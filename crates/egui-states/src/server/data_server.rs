use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock, RwLockWriteGuard};

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
    item_size: usize,
    value: RwLock<(Vec<u8>, usize)>,
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
            item_size: data_type.item_size(),
            value: RwLock::new((Vec::new(), 0)),
            sender,
            connected,
            event: Event::new(),
        })
    }

    pub(crate) fn set(&self, data: DataHolder, update: bool) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let mut w = self.value.write();
        let slice = unsafe { std::slice::from_raw_parts(data.data as *const u8, data.data_size) };
        w.0.clear();
        w.0.extend_from_slice(slice);
        w.1 = data.count;

        let r = RwLockWriteGuard::downgrade(w);
        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Set(count);
            let messages = pack_data(
                self.id,
                &r.0,
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
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let mut w = self.value.write();
        let original_len = w.0.len();
        w.0.extend_from_slice(slice);
        w.1 += data.count;
        let r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Add(count);
            let messages = pack_data(
                self.id,
                &r.0[original_len..],
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
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let mut w = self.value.write();
        if index + data.count > w.1 {
            return Err(format!(
                "Replace range out of bounds: index {} + data count {} exceeds current size {}",
                index, data.count, w.1
            ));
        }
        let byte_index = index * self.item_size;
        w.0[byte_index..byte_index + data.data_size].copy_from_slice(slice);
        let r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Replace(index as u64, count);
            let messages = pack_data(
                self.id,
                &r.0[byte_index..byte_index + data.data_size],
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
        if index + size > w.1 {
            return Err(format!(
                "Remove range out of bounds: end {} exceeds current size {}",
                index + size,
                w.1
            ));
        }
        let byte_index = index * self.item_size;
        w.0.drain(byte_index..byte_index + size * self.item_size);
        w.1 -= size;
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
        w.0.clear();
        w.1 = 0;

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
        f(&value.0)
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

        let count = r.0.len() / self.data_type.item_size();
        if count == 0 {
            let header = DataHeader::Clear(false);
            let message = header.serialize(self.id, false).map_err(|_| ())?;
            self.sender.send(message);
            self.event.set();
        } else {
            let transport_type = TransportType::Set(count as u64);
            let messages = pack_data(
                self.id,
                &r.0,
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

            let mut guard = self.lock.lock();
            if cache {
                *guard = Some((slice.to_vec(), data.count as usize));
            } else {
                *guard = None;
            }

            match blocking {
                true => self.event.wait_clear(),
                false => self.event.wait(),
            }
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            for (message, single) in messages {
                self.sender.send_set(message, single);
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
        let mut g = self.lock.lock();
        match g.take() {
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

// DataMulti --------------------------------------------------
pub(crate) struct DataMulti {
    pub(crate) name: String,
    id: u64,
    pub(crate) data_type: DataType,
    item_size: usize,
    values: RwLock<NoHashMap<u32, (Vec<u8>, usize)>>,
    sync_counter: Mutex<usize>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
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
            item_size: data_type.item_size(),
            values: RwLock::new(NoHashMap::default()),
            sync_counter: Mutex::new(0),
            sender,
            connected,
            event: Event::new(),
        })
    }

    pub(crate) fn remove_index(&self, index: u32, update: bool) -> Result<(), String> {
        let mut w = self.values.write();
        if let Some(_) = w.remove(&index) {
            let _r = RwLockWriteGuard::downgrade(w);
            if self.connected.load(std::sync::atomic::Ordering::Relaxed) {
                let header = MultiDataHeader::Remove(index, update);
                let message = header
                    .serialize(self.id)
                    .map_err(|_| format!("Failed to serialize remove index header"))?;
                self.sender.send(message);
            }
        }

        Ok(())
    }

    pub(crate) fn reset(&self, update: bool) -> Result<(), String> {
        let mut w = self.values.write();
        if !w.is_empty() {
            w.clear();

            let _r = RwLockWriteGuard::downgrade(w);
            if self.connected.load(std::sync::atomic::Ordering::Relaxed) {
                let header = MultiDataHeader::Reset(update);
                let message = header
                    .serialize(self.id)
                    .map_err(|_| format!("Failed to serialize reset header"))?;
                self.sender.send(message);
            }
        }

        Ok(())
    }

    pub(crate) fn set(&self, index: u32, data: DataHolder, update: bool) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let mut w = self.values.write();
        match w.get_mut(&index) {
            Some((vec, size)) => {
                vec.clear();
                let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
                vec.extend_from_slice(slice);
                *size = data.count;
            }
            None => {
                let mut vec = Vec::with_capacity(data.data_size);
                unsafe {
                    vec.set_len(data.data_size);
                    std::ptr::copy_nonoverlapping(data.data, vec.as_mut_ptr(), data.data_size);
                }
                w.insert(index, (vec, data.count));
            }
        }

        let r = RwLockWriteGuard::downgrade(w);
        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Set(count);
            let entry = r
                .get(&index)
                .expect("DataMulti index was just inserted but is missing");
            let messages = pack_data(
                self.id,
                &entry.0,
                transport_type,
                count,
                self.data_type,
                Some(index),
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

    pub(crate) fn add(&self, index: u32, data: DataHolder, update: bool) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let mut w = self.values.write();
        let entry = w
            .get_mut(&index)
            .ok_or_else(|| format!("DataMulti index {} does not exist", index))?;
        let original_len = entry.0.len();
        entry.0.extend_from_slice(slice);
        entry.1 += data.count;
        let r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Add(count);
            let entry = r
                .get(&index)
                .expect("DataMulti index was just inserted but is missing");
            let messages = pack_data(
                self.id,
                &entry.0[original_len..],
                transport_type,
                count,
                self.data_type,
                Some(index),
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
        index: u32,
        data_index: usize,
        data: DataHolder,
        update: bool,
    ) -> Result<(), String> {
        check_data_type(&data, self.data_type, self.item_size)?;

        let slice = unsafe { std::slice::from_raw_parts(data.data, data.data_size) };
        let mut w = self.values.write();
        let value = w
            .get_mut(&index)
            .ok_or_else(|| format!("DataMulti index {} does not exist", index))?;
        if data_index + data.count > value.1 {
            return Err(format!(
                "Replace range out of bounds for DataMulti index {}: data_index {} + data count {} exceeds current size {}",
                index, data_index, data.count, value.1
            ));
        }
        let byte_index = data_index * self.item_size;
        value.0[byte_index..byte_index + data.data_size].copy_from_slice(slice);
        let r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let count = data.count as u64;
            let transport_type = TransportType::Replace(data_index as u64, count);
            let entry = r
                .get(&index)
                .ok_or_else(|| format!("DataMulti index {} does not exist", index))?;
            let messages = pack_data(
                self.id,
                &entry.0[byte_index..byte_index + data.data_size],
                transport_type,
                count,
                self.data_type,
                Some(index),
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

    pub(crate) fn remove(
        &self,
        index: u32,
        data_index: usize,
        size: usize,
        update: bool,
    ) -> Result<(), String> {
        if size == 0 {
            return Err("Invalid remove size: size must be greater than 0".to_string());
        }

        let mut w = self.values.write();
        let value = w
            .get_mut(&index)
            .ok_or_else(|| format!("DataMulti index {} does not exist", index))?;
        if data_index + size > value.1 {
            return Err(format!(
                "Remove range out of bounds for DataMulti index {}: data_index {} + size {} exceeds current size {}",
                index, data_index, size, value.1
            ));
        }
        let byte_index = data_index * self.item_size;
        value
            .0
            .drain(byte_index..byte_index + size * self.item_size);
        value.1 -= size;
        let _r = RwLockWriteGuard::downgrade(w);

        if self.connected.load(Ordering::Acquire) {
            let header = DataHeader::Drain(data_index as u64, size as u64, update);
            let message = MultiDataHeader::serialize_modify(self.id, index, header)
                .map_err(|_| "Failed to serialize header".to_string())?;

            self.event.wait_clear();
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn clear(&self, index: u32, update: bool) -> Result<(), String> {
        let mut w = self.values.write();
        let value = w
            .get_mut(&index)
            .ok_or_else(|| "Index not found".to_string())?;
        value.0.clear();
        value.1 = 0;

        let _r = RwLockWriteGuard::downgrade(w);
        if self.connected.load(Ordering::Acquire) {
            let header = MultiDataHeader::Modify(index, DataHeader::Clear(update));
            let message = header
                .serialize(self.id)
                .map_err(|_| "Failed to serialize header".to_string())?;

            self.sender.send(message);
        }

        Ok(())
    }

    pub(crate) fn get<R>(&self, key: u32, f: impl Fn(Option<&[u8]>) -> R) -> R {
        f(self
            .values
            .read()
            .get(&key)
            .map(|(data, _)| data.as_slice()))
    }
}

impl Acknowledge for DataMulti {
    fn acknowledge(&self) {
        let mut w = self.sync_counter.lock();
        if *w > 0 {
            *w = w.saturating_sub(1);
            if *w > 0 {
                return;
            }
        }

        self.event.set();
    }
}

impl SyncTrait for DataMulti {
    fn sync(&self) -> Result<(), ()> {
        let r = self.values.read();
        if r.is_empty() {
            let message = MultiDataHeader::Reset(false).serialize(self.id)?;
            self.sender.send(message);
            self.event.set();
        } else {
            self.event.clear();
            let mut w = self.sync_counter.lock();
            *w = r.len();
            for (index, (data, count)) in r.iter() {
                let transport_type = TransportType::Set(*count as u64);
                let messages = pack_data(
                    self.id,
                    data,
                    transport_type,
                    *count as u64,
                    self.data_type,
                    Some(*index),
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

// functions ------------------------------------------------
fn check_data_type(data: &DataHolder, data_type: DataType, item_size: usize) -> Result<(), String> {
    if data.data_type != data_type {
        return Err(format!(
            "Data type mismatch: expected {:?}, got {:?}",
            data_type, data.data_type
        ));
    }

    if data.data_size % item_size != 0 {
        return Err(format!(
            "Data size must be a multiple of element size: expected multiple of {}, got {}",
            item_size, data.data_size
        ));
    }

    Ok(())
}

fn pack_data(
    id: u64,
    data: &[u8],
    transport_type: TransportType,
    data_count: u64,
    data_type: DataType,
    multi_data: Option<u32>,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let mut messages = Vec::with_capacity(1);

    if data.len() <= MSG_SIZE_THRESHOLD {
        let header = DataHeader::All(data_type, transport_type, update, data.len() as u32);
        let mut message = match multi_data {
            None => header.serialize(id, true),
            Some(index) => MultiDataHeader::serialize_modify(id, index, header),
        }
        .map_err(|_| "Failed to serialize header".to_string())?;
        message.reserve_exact(data.len());
        message.extend_from_slice(&data);
        messages.push((message, true));
    } else {
        let element_size = data_type.item_size();
        let chunk_size = MSG_SIZE_THRESHOLD / element_size * element_size;
        let mut processed = 0;
        let header = DataHeader::StartBatch(data_count, chunk_size as u32);
        let mut message = match multi_data {
            None => header.serialize(id, true),
            Some(index) => MultiDataHeader::serialize_modify(id, index, header),
        }
        .map_err(|_| "Failed to serialize header".to_string())?;
        message.reserve_exact(chunk_size);
        message.extend_from_slice(&data[..chunk_size]);
        messages.push((message, true));
        processed += chunk_size;

        while processed < data.len() {
            let remaining = data.len() - processed;
            if remaining <= chunk_size {
                let header = DataHeader::End(data_type, transport_type, update, remaining as u32);
                let mut message = match multi_data {
                    None => header.serialize(id, true),
                    Some(index) => MultiDataHeader::serialize_modify(id, index, header),
                }
                .map_err(|_| "Failed to serialize header".to_string())?;
                message.extend_from_slice(&data[processed..]);
                messages.push((message, false));
                break;
            }

            let header = DataHeader::Batch(chunk_size as u32);
            let mut message = match multi_data {
                None => header.serialize(id, true),
                Some(index) => MultiDataHeader::serialize_modify(id, index, header),
            }
            .map_err(|_| "Failed to serialize header".to_string())?;
            message.reserve_exact(chunk_size);
            message.extend_from_slice(&data[processed..processed + chunk_size]);
            messages.push((message, true));
            processed += chunk_size;
        }
    }

    Ok(messages)
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
