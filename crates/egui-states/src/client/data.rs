use bytes::Bytes;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::serialization::{MessageData, deserialize, to_message};

pub(crate) struct DataMessageAll {
    pub type_id: u32,
    pub is_add: bool,
    pub header: Bytes,
    pub data: Bytes,
}

pub(crate) struct DataMessageHead {
    pub type_id: u32,
    pub data_size_all: u64,
    pub header: Bytes,
    pub data: Bytes,
}

pub(crate) struct DataMessageEnd {
    pub is_add: bool,
    pub data: Bytes,
}

pub(crate) enum DataMessage {
    All(DataMessageAll),
    Head(DataMessageHead),
    Data(Bytes),
    End(DataMessageEnd),
}

pub(crate) struct ChannelMessageData {
    pub is_add: bool,
    pub header: MessageData,
    pub data: Vec<u8>,
}

pub(crate) trait UpdateData: Sync + Send {
    fn update_data(&self, message: DataMessage) -> Result<(), String>;
}

pub struct Data<T> {
    name: String,
    id: u64,
    type_id: u32,
    inner: Arc<RwLock<(T, Vec<u8>, bool)>>,
    buffer: Arc<Mutex<Option<(T, Vec<u8>)>>>,
    sender: MessageSender,
}

impl<T> Data<T>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        header: T,
        sender: MessageSender,
    ) -> Self {
        Self {
            name,
            id,
            type_id,
            inner: Arc::new(RwLock::new((header, Vec::new(), false))),
            buffer: Arc::new(Mutex::new(None)),
            sender,
        }
    }

    pub fn get(&self) -> (T, Vec<u8>) {
        let inner = self.inner.read();
        (inner.0.clone(), inner.1.clone())
    }

    pub fn get_updated(&self) -> Option<(T, Vec<u8>)> {
        let mut inner = self.inner.write();
        if inner.2 {
            inner.2 = false;
            Some((inner.0.clone(), inner.1.clone()))
        } else {
            None
        }
    }

    pub fn read<R>(&self, f: impl Fn((&T, &Vec<u8>, bool)) -> R) -> R {
        let mut inner = self.inner.write();
        let result = f((&inner.0, &inner.1, inner.2));
        inner.2 = false;
        result
    }
}

impl<T> Clone for Data<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            inner: self.inner.clone(),
            buffer: self.buffer.clone(),
            sender: self.sender.clone(),
        }
    }
}

impl<T> UpdateData for Data<T>
where
    T: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_data(&self, message: DataMessage) -> Result<(), String> {
        match message {
            DataMessage::All(msg) => {
                if self.type_id != msg.type_id {
                    self.sender.send(ChannelMessage::Ack(self.id));
                    return Err(format!("Type id mismatch for Data: {}", self.name));
                }
                let value = deserialize(&msg.header).map_err(|e| {
                    self.sender.send(ChannelMessage::Ack(self.id));
                    format!("Failed to deserialize header: {}", e)
                })?;
                if msg.is_add {
                    let mut w = self.inner.write();
                    w.0 = value;
                    w.1.extend_from_slice(&msg.data);
                    w.2 = true;
                } else {
                    let mut w = self.inner.write();
                    *w = (value, msg.data.to_vec(), true);
                }
                self.sender.send(ChannelMessage::Ack(self.id));
                Ok(())
            }
            DataMessage::Head(msg) => {
                if self.type_id != msg.type_id {
                    self.sender.send(ChannelMessage::Ack(self.id));
                    return Err(format!("Type id mismatch for Data: {}", self.name));
                }
                let value = deserialize(&msg.header).map_err(|e| {
                    self.sender.send(ChannelMessage::Ack(self.id));
                    format!("Failed to deserialize header: {}", e)
                })?;
                let mut buffer = Vec::with_capacity(msg.data_size_all as usize);
                buffer.extend_from_slice(&msg.data);
                *self.buffer.lock() = Some((value, buffer));
                Ok(())
            }
            DataMessage::Data(data) => {
                let mut b = self.buffer.lock();
                if let Some((_, buffer)) = b.as_mut() {
                    buffer.extend_from_slice(&data);
                    Ok(())
                } else {
                    self.sender.send(ChannelMessage::Ack(self.id));
                    Err(format!(
                        "No header found for Data: {} when updating data",
                        self.name
                    ))
                }
            }
            DataMessage::End(msg) => {
                let taken = self.buffer.lock().take();
                let (header, mut buffer) = taken.ok_or_else(|| {
                    self.sender.send(ChannelMessage::Ack(self.id));
                    format!("No header found for Data: {} when updating end", self.name)
                })?;
                buffer.extend_from_slice(&msg.data);
                if msg.is_add {
                    let mut w = self.inner.write();
                    w.0 = header;
                    w.1.extend_from_slice(&buffer);
                    w.2 = true;
                } else {
                    let mut w = self.inner.write();
                    *w = (header, buffer, true);
                }
                self.sender.send(ChannelMessage::Ack(self.id));
                Ok(())
            }
        }
    }
}

// DataStatic --------------------------------------------
pub struct DataStatic<T> {
    name: String,
    id: u64,
    type_id: u32,
    inner: Arc<RwLock<(T, Vec<u8>, bool)>>,
    buffer: Arc<Mutex<Option<(T, Vec<u8>)>>>,
}

impl<T: Clone> DataStatic<T> {
    pub(crate) fn new(name: String, id: u64, type_id: u32, header: T) -> Self {
        Self {
            name,
            id,
            type_id,
            inner: Arc::new(RwLock::new((header, Vec::new(), false))),
            buffer: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get(&self) -> (T, Vec<u8>) {
        let inner = self.inner.read();
        (inner.0.clone(), inner.1.clone())
    }

    pub fn get_updated(&self) -> Option<(T, Vec<u8>)> {
        let mut inner = self.inner.write();
        if inner.2 {
            inner.2 = false;
            Some((inner.0.clone(), inner.1.clone()))
        } else {
            None
        }
    }

    pub fn read<R>(&self, f: impl Fn((&T, &Vec<u8>, bool)) -> R) -> R {
        let inner = self.inner.read();
        f((&inner.0, &inner.1, inner.2))
    }
}

impl<T> Clone for DataStatic<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            type_id: self.type_id,
            inner: self.inner.clone(),
            buffer: self.buffer.clone(),
        }
    }
}

impl<T> UpdateData for DataStatic<T>
where
    T: for<'a> Deserialize<'a> + Send + Sync,
{
    fn update_data(&self, message: DataMessage) -> Result<(), String> {
        match message {
            DataMessage::All(msg) => {
                if self.type_id != msg.type_id {
                    return Err(format!("Type id mismatch for DataStatic: {}", self.name));
                }
                let value = deserialize(&msg.header)
                    .map_err(|e| format!("Failed to deserialize header: {}", e))?;
                if msg.is_add {
                    let mut w = self.inner.write();
                    w.0 = value;
                    w.1.extend_from_slice(&msg.data);
                    w.2 = true;
                } else {
                    let mut w = self.inner.write();
                    *w = (value, msg.data.to_vec(), true);
                }
                Ok(())
            }
            DataMessage::Head(msg) => {
                if self.type_id != msg.type_id {
                    return Err(format!("Type id mismatch for DataStatic: {}", self.name));
                }
                let value = deserialize(&msg.header)
                    .map_err(|e| format!("Failed to deserialize header: {}", e))?;
                let mut buffer = Vec::with_capacity(msg.data_size_all as usize);
                buffer.extend_from_slice(&msg.data);
                *self.buffer.lock() = Some((value, buffer));
                Ok(())
            }
            DataMessage::Data(data) => {
                let mut b = self.buffer.lock();
                if let Some((_, buffer)) = b.as_mut() {
                    buffer.extend_from_slice(&data);
                    Ok(())
                } else {
                    Err(format!(
                        "No header found for DataStatic: {} when updating data",
                        self.name
                    ))
                }
            }
            DataMessage::End(msg) => {
                let taken = self.buffer.lock().take();
                let (header, mut buffer) = taken.ok_or_else(|| {
                    format!(
                        "No header found for DataStatic: {} when updating end",
                        self.name
                    )
                })?;
                buffer.extend_from_slice(&msg.data);
                if msg.is_add {
                    let mut w = self.inner.write();
                    w.0 = header;
                    w.1.extend_from_slice(&buffer);
                    w.2 = true;
                } else {
                    let mut w = self.inner.write();
                    *w = (header, buffer, true);
                }
                Ok(())
            }
        }
    }
}
