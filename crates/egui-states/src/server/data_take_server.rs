use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::data_transport::{DataMultiTakeHeader, DataTakeHeader, DataType};
use crate::hashing::NoHashMap;
use crate::serialization::{FastVec, MSG_SIZE_THRESHOLD};
use crate::server::data_server::{DataHolder, check_data_type};
use crate::server::event::Event;
use crate::server::sender::MessageSender;
use crate::server::server::{Acknowledge, SyncTrait};

// DataTake --------------------------------------------------
pub(crate) struct DataTake {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    event: Event,
    lock: Mutex<Option<(Vec<u8>, usize)>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl DataTake {
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
            item_size: data_type.item_size(),
            event: Event::new(),
            lock: Mutex::new(None),
            sender,
            connected,
        })
    }

    pub(crate) fn set(
        &self,
        data: DataHolder,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data as *const u8, data.data_size) };

        if self.connected.load(Ordering::Acquire) {
            let messages = pack_data_take(
                self.id,
                slice,
                data.count as u64,
                self.data_type,
                blocking,
                update,
            )?;

            match blocking {
                true => self.event.wait_clear(), // ← clear flag for next call to block
                false => self.event.wait(),      // ← leave flag set, next call sends immediately
            }

            let mut guard = self.lock.lock();
            if cache {
                *guard = Some((slice.to_vec(), data.count as usize));
            } else {
                *guard = None;
            }

            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        } else {
            let mut guard = self.lock.lock();
            if cache {
                *guard = Some((slice.to_vec(), data.count as usize));
            } else {
                *guard = None;
            }
        }

        Ok(())
    }
}

impl Acknowledge for DataTake {
    fn acknowledge(&self) {
        self.event.set();
    }
}

impl SyncTrait for DataTake {
    fn sync(&self) -> Result<(), ()> {
        let g = self.lock.lock();
        match g.clone() {
            Some((data, count)) => {
                let messages =
                    pack_data_take(self.id, &data, count as u64, self.data_type, false, false)
                        .map_err(|_| ())?;

                self.event.clear();
                for (message, single) in messages {
                    self.sender.send_set(message, single);
                }
            }
            None => {
                self.event.set();
            }
        }

        Ok(())
    }
}

// DataMultiTake --------------------------------------------------
pub(crate) struct DataMultiTake {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    values: RwLock<NoHashMap<u32, (Vec<u8>, usize)>>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl DataMultiTake {
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
            item_size: data_type.item_size(),
            values: RwLock::new(NoHashMap::default()),
            sender,
            connected,
            event: Event::new(),
        })
    }

    pub(crate) fn set(
        &self,
        index: u32,
        data: DataHolder,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data as *const u8, data.data_size) };

        if self.connected.load(Ordering::Acquire) {
            let messages = pack_data_multi_take(
                self.id,
                index,
                slice,
                data.count as u64,
                self.data_type,
                blocking,
                update,
            )?;

            match blocking {
                true => self.event.wait_clear(),
                false => self.event.wait(),
            }

            {
                let mut w = self.values.write();
                if cache {
                    w.insert(index, (slice.to_vec(), data.count));
                } else {
                    w.remove(&index);
                }
            }

            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            for (message, single) in messages {
                self.sender.send_set(message, single);
            }
        } else if cache {
            self.values
                .write()
                .insert(index, (slice.to_vec(), data.count));
        } else {
            self.values.write().remove(&index);
        }

        Ok(())
    }

    pub(crate) fn remove_index(&self, index: u32, update: bool) -> Result<(), String> {
        self.values.write().remove(&index);
        if self.connected.load(Ordering::Acquire) {
            let header = DataMultiTakeHeader::Remove(index, update);
            let message = header
                .serialize(self.id)
                .map_err(|_| "Failed to serialize remove index header".to_string())?;
            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn reset(&self, update: bool) -> Result<(), String> {
        self.values.write().clear();
        if self.connected.load(Ordering::Acquire) {
            let header = DataMultiTakeHeader::Reset(update);
            let message = header
                .serialize(self.id)
                .map_err(|_| "Failed to serialize reset header".to_string())?;
            self.sender.send(message);
        }

        Ok(())
    }
}

impl Acknowledge for DataMultiTake {
    fn acknowledge(&self) {
        self.event.set();
    }
}

impl SyncTrait for DataMultiTake {
    fn sync(&self) -> Result<(), ()> {
        let r = self.values.read();
        if r.is_empty() {
            let message = DataMultiTakeHeader::Reset(false).serialize(self.id)?;
            self.sender.send(message);
            self.event.set();
        } else {
            self.event.clear();
            for (index, (data, count)) in r.iter() {
                let messages = pack_data_multi_take(
                    self.id,
                    *index,
                    data,
                    *count as u64,
                    self.data_type,
                    false,
                    false,
                )
                .map_err(|_| ())?;

                for (message, single) in messages {
                    self.sender.send_set(message, single);
                }

                if !self.connected.load(Ordering::Acquire) {
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

fn pack_data_take(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let mut messages = Vec::with_capacity(1);

    if data.len() <= MSG_SIZE_THRESHOLD {
        let header = crate::data_transport::DataTakeHeader::All(
            data_type,
            data_count,
            update,
            data.len() as u32,
        );
        let mut message = header
            .serialize(id, blocking, true)
            .map_err(|_| "Failed to serialize DataTake header".to_string())?;
        message.reserve_exact(data.len());
        message.extend_from_slice(&data);
        messages.push((message, true));
    } else {
        let element_size = data_type.item_size();
        let chunk_size = MSG_SIZE_THRESHOLD / element_size * element_size;
        let mut processed = 0;
        let header =
            crate::data_transport::DataTakeHeader::StartBatch(data_count, chunk_size as u32);
        let mut message = header
            .serialize(id, blocking, true)
            .map_err(|_| "Failed to serialize DataTake header".to_string())?;
        message.reserve_exact(chunk_size);
        message.extend_from_slice(&data[..chunk_size]);
        messages.push((message, true));
        processed += chunk_size;

        while processed < data.len() {
            let remaining = data.len() - processed;
            if remaining <= chunk_size {
                let header = crate::data_transport::DataTakeHeader::End(
                    data_type,
                    data_count,
                    update,
                    remaining as u32,
                );
                let mut message = header
                    .serialize(id, blocking, true)
                    .map_err(|_| "Failed to serialize DataTake header".to_string())?;
                message.extend_from_slice(&data[processed..]);
                messages.push((message, false));
                break;
            }

            let header = crate::data_transport::DataTakeHeader::Batch(chunk_size as u32);
            let mut message = header
                .serialize(id, blocking, true)
                .map_err(|_| "Failed to serialize DataTake header".to_string())?;
            message.reserve_exact(chunk_size);
            message.extend_from_slice(&data[processed..processed + chunk_size]);
            messages.push((message, true));
            processed += chunk_size;
        }
    }

    Ok(messages)
}

fn pack_data_multi_take(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let mut messages = Vec::with_capacity(1);

    if data.len() <= MSG_SIZE_THRESHOLD {
        let header = DataTakeHeader::All(data_type, data_count, update, data.len() as u32);
        let mut message = DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
            .map_err(|_| "Failed to serialize DataMultiTake header".to_string())?;
        message.reserve_exact(data.len());
        message.extend_from_slice(&data);
        messages.push((message, true));
    } else {
        let element_size = data_type.item_size();
        let chunk_size = MSG_SIZE_THRESHOLD / element_size * element_size;
        let mut processed = 0;
        let header = DataTakeHeader::StartBatch(data_count, chunk_size as u32);
        let mut message = DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
            .map_err(|_| "Failed to serialize DataMultiTake header".to_string())?;
        message.reserve_exact(chunk_size);
        message.extend_from_slice(&data[..chunk_size]);
        messages.push((message, true));
        processed += chunk_size;

        while processed < data.len() {
            let remaining = data.len() - processed;
            if remaining <= chunk_size {
                let header = DataTakeHeader::End(data_type, data_count, update, remaining as u32);
                let mut message =
                    DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
                        .map_err(|_| "Failed to serialize DataMultiTake header".to_string())?;
                message.extend_from_slice(&data[processed..]);
                messages.push((message, false));
                break;
            }

            let header = DataTakeHeader::Batch(chunk_size as u32);
            let mut message = DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
                .map_err(|_| "Failed to serialize DataMultiTake header".to_string())?;
            message.reserve_exact(chunk_size);
            message.extend_from_slice(&data[processed..processed + chunk_size]);
            messages.push((message, true));
            processed += chunk_size;
        }
    }

    Ok(messages)
}
