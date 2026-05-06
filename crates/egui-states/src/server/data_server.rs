use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{RwLock, RwLockWriteGuard};

use crate::data_transport::{DataHeader, DataType, MultiDataHeader, TransportType};
use crate::hashing::NoHashMap;
use crate::serialization::{FastVec, MSG_SIZE_THRESHOLD};
use crate::server::event::Event;
use crate::server::sender::MessageSender;
use crate::server::server::{Acknowledge, SyncTrait};

pub(crate) struct DataHolder {
    pub data: *const u8,
    pub count: usize,
    pub data_size: usize,
    pub data_type: DataType,
}

unsafe impl Send for DataHolder {}
unsafe impl Sync for DataHolder {}

// Data --------------------------------------------------
pub(crate) struct Data {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    value: RwLock<Vec<u8>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl Data {
    pub(crate) fn new(
        name: String,
        id: u64,
        data_type: DataType,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            data_type,
            value: RwLock::new(Vec::new()),
            sender,
            connected,
            event: Event::new(),
        })
    }

    fn check_data_type(&self, data: &DataHolder) -> Result<(), String> {
        if data.data_type != self.data_type {
            return Err(format!(
                "Data type mismatch: expected {:?}, got {:?}",
                self.data_type, data.data_type
            ));
        }

        if data.data_size % self.data_type.element_size() != 0 {
            return Err(format!(
                "Data size must be a multiple of element size: expected multiple of {}, got {}",
                self.data_type.element_size(),
                data.data_size
            ));
        }

        Ok(())
    }

    pub(crate) fn set(&self, data: DataHolder, update: bool) -> Result<(), String> {
        self.check_data_type(&data)?;

        let mut vec = Vec::with_capacity(data.data_size);
        unsafe {
            vec.set_len(data.data_size);
            std::ptr::copy_nonoverlapping(data.data as *const u8, vec.as_mut_ptr(), data.data_size);
        }

        let mut w = self.value.write();
        *w = vec;
        let r = RwLockWriteGuard::downgrade(w);
        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Set(count);
            let messages = pack_data(
                self.id,
                &r,
                transport_type,
                count,
                self.data_type,
                None,
                update,
            )?;

            self.event.wait_clear();
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        }

        Ok(())
    }

    pub(crate) fn add(&self, data: DataHolder, update: bool) -> Result<(), String> {
        self.check_data_type(&data)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let mut w = self.value.write();
        let original_len = w.len();
        w.extend_from_slice(slice);
        let r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Add(count);
            let messages = pack_data(
                self.id,
                &r[original_len..],
                transport_type,
                count,
                self.data_type,
                None,
                update,
            )?;

            self.event.wait_clear();
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        }

        Ok(())
    }

    pub(crate) fn replace(
        &self,
        data: DataHolder,
        index: usize,
        update: bool,
    ) -> Result<(), String> {
        self.check_data_type(&data)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let mut w = self.value.write();
        if index + data.data_size > w.len() {
            return Err(format!(
                "Replace range out of bounds: index {} + data size {} exceeds current size {}",
                index,
                data.data_size,
                w.len()
            ));
        }
        w[index..index + data.data_size].copy_from_slice(slice);
        let r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Replace(index as u64, count);
            let messages = pack_data(
                self.id,
                &r[index..index + data.data_size],
                transport_type,
                count,
                self.data_type,
                None,
                update,
            )?;

            self.event.wait_clear();
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        }

        Ok(())
    }

    pub(crate) fn remove(&self, index: usize, size: usize, update: bool) -> Result<(), String> {
        if size == 0 {
            return Err("Invalid remove size: size must be greater than 0".to_string());
        }

        let mut w = self.value.write();
        if index + size > w.len() * self {
            return Err(format!(
                "Remove range out of bounds: end {} exceeds current size {}",
                index + size,
                w.len()
            ));
        }
        w.drain(index..index + size);
        let _r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let header = DataHeader::Drain(index as u64, size as u64, update);
            let message = header
                .serialize(self.id, false)
                .map_err(|_| "Failed to serialize header".to_string())?;

            self.event.wait_clear();
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn clear(&self, update: bool) -> Result<(), String> {
        let mut w = self.value.write();
        w.clear();
        let _r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let header = DataHeader::Clear(update);
            let message = header
                .serialize(self.id, false)
                .map_err(|_| "Failed to serialize header".to_string())?;

            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn get<R>(&self, f: impl Fn(&[u8]) -> R) -> R {
        let value = self.value.read();
        f(&value)
    }
}

impl Acknowledge for Data {
    fn acknowledge(&self) {
        self.event.set();
    }
}

impl SyncTrait for Data {
    fn sync(&self) -> Result<(), ()> {
        let r = self.value.read();

        let count = r.len() / self.data_type.element_size();
        if count == 0 {
            let header = DataHeader::Clear(false);
            let message = header.serialize(self.id, false).map_err(|_| ())?;
            self.sender.send(message);
            self.event.set();
        } else {
            let transport_type = TransportType::Set(count as u64);
            let messages = pack_data(
                self.id,
                &r,
                transport_type,
                count as u64,
                self.data_type,
                None,
                false,
            )
            .map_err(|_| ())?;

            self.event.clear();
            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        }

        Ok(())
    }
}

// DataMulti --------------------------------------------------
pub(crate) struct DataMulti {
    pub(crate) name: String,
    id: u64,
    data_type: DataType,
    values: RwLock<NoHashMap<u32, Vec<u8>>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl DataMulti {
    pub(crate) fn new(
        name: String,
        id: u64,
        data_type: DataType,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            data_type,
            values: RwLock::new(NoHashMap::default()),
            sender,
            connected,
        })
    }

    pub(crate) fn get(&self, key: u32) -> Option<Vec<u8>> {
        self.values.read().get(&key).cloned()
    }

    pub(crate) fn remove(&self, key: u32) {
        let mut w = self.values.write();
        w.remove(&key);
    }

    pub(crate) fn clear(&self) {
        let mut w = self.values.write();
        w.clear();
    }
}

// functions ------------------------------------------------
fn pack_data(
    id: u64,
    data: &[u8],
    transport_type: TransportType,
    data_count: u64,
    data_type: DataType,
    modify: Option<u32>,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let count = data.len() / MSG_SIZE_THRESHOLD + 1;
    let mut messages = Vec::with_capacity(count);

    if count == 1 {
        let header = DataHeader::All(data_type, transport_type, update, data.len() as u32);
        let mut message = match modify {
            None => header.serialize(id, true),
            Some(index) => MultiDataHeader::serialize_modify(id, index, header),
        }
        .map_err(|_| "Failed to serialize header".to_string())?;
        message.reserve_exact(data.len());
        message.extend_from_slice(&data);
        messages.push((message, true));
    } else {
        let mut processed = 0;
        let header = DataHeader::StartBatch(data_count, MSG_SIZE_THRESHOLD as u32);
        let mut message = match modify {
            None => header.serialize(id, true),
            Some(index) => MultiDataHeader::serialize_modify(id, index, header),
        }
        .map_err(|_| "Failed to serialize header".to_string())?;
        message.reserve_exact(MSG_SIZE_THRESHOLD);
        message.extend_from_slice(&data[..MSG_SIZE_THRESHOLD]);
        messages.push((message, true));
        processed += MSG_SIZE_THRESHOLD;

        while processed < data.len() {
            let remaining = data.len() - processed;
            if remaining <= MSG_SIZE_THRESHOLD {
                let header = DataHeader::End(data_type, transport_type, update, remaining as u32);
                let mut message = match modify {
                    None => header.serialize(id, true),
                    Some(index) => MultiDataHeader::serialize_modify(id, index, header),
                }
                .map_err(|_| "Failed to serialize header".to_string())?;
                message.extend_from_slice(&data[processed..]);
                messages.push((message, false));
                break;
            }

            let header = DataHeader::Batch(MSG_SIZE_THRESHOLD as u32);
            let mut message = match modify {
                None => header.serialize(id, true),
                Some(index) => MultiDataHeader::serialize_modify(id, index, header),
            }
            .map_err(|_| "Failed to serialize header".to_string())?;
            message.reserve_exact(MSG_SIZE_THRESHOLD);
            message.extend_from_slice(&data[processed..processed + MSG_SIZE_THRESHOLD]);
            messages.push((message, true));
            processed += MSG_SIZE_THRESHOLD;
        }
    }

    Ok(messages)
}
