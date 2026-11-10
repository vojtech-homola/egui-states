use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, MutexGuard, RwLock};

use crate::event::Event;
use crate::hashing::NoHashMap;
use crate::protocol::image::{
    ImageHeader, ImageMultiHeader, ImageType, checked_image_bytes, checked_image_shape,
    serialize_image_multi_header,
};
use crate::protocol::{FastVec, MSG_SIZE_THRESHOLD, TRANSFER_CHUNK_SIZE};
use crate::server_core::error::CoreError;
use crate::server_core::net::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::states::image::common::{
    ImageData, checked_fill_size, checked_image_rect, checked_image_size, pack_image_set,
    pack_image_update, store_rgba, write_rectangle,
};

enum Buffer {
    Set(Vec<(FastVec<32>, bool)>),
    Update(u32, [usize; 4], VecDeque<(FastVec<32>, bool)>),
}

struct TransferState {
    buffer: Buffer,
    sync_generation: u64,
    sync_acks: usize,
}

struct ImageMultiTransfer {
    id: u64,
    lock: Mutex<()>,
    state: Mutex<TransferState>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl ImageMultiTransfer {
    fn new(id: u64, sender: MessageSender, connected: Arc<AtomicBool>) -> Self {
        let event = Event::new();
        event.set();
        Self {
            id,
            lock: Mutex::new(()),
            state: Mutex::new(TransferState {
                buffer: Buffer::Set(Vec::new()),
                sync_generation: 0,
                sync_acks: 0,
            }),
            sender,
            connected,
            event,
        }
    }

    #[inline]
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// The generation of the current connection. Producers read it while they
    /// hold the image storage, so a reset after their `sync_required` check
    /// invalidates their work.
    fn generation(&self) -> u64 {
        self.state.lock().sync_generation
    }

    #[cfg(test)]
    #[inline]
    fn is_idle(&self) -> bool {
        self.event.is_set()
    }

    fn send_or_buffer_set(
        &self,
        _lock: MutexGuard<'_, ()>,
        index: u32,
        to_send: Vec<(FastVec<32>, bool)>,
        generation: u64,
    ) -> Result<(), String> {
        let mut state = self.state.lock();
        // A reset after the producer checked `sync_required` belongs to a
        // later connection, so this work is stale.
        if state.sync_generation != generation {
            return Ok(());
        }
        match state.buffer {
            Buffer::Set(ref mut data) => {
                if data.is_empty() {
                    if self.event.is_set() {
                        self.event.clear();
                        self.send_all(to_send);
                    } else {
                        data.extend(to_send);
                    }
                    return Ok(());
                }
            }
            Buffer::Update(saved_index, _, _) => {
                if self.event.is_set() {
                    self.event.clear();
                    self.send_all(to_send);
                    return Ok(());
                }

                // A complete set supersedes pending updates only for the same
                // image. Dropping another key's chunks would leave that client
                // texture only partially updated.
                if saved_index == index {
                    state.buffer = Buffer::Set(to_send);
                    return Ok(());
                }
            }
        }

        drop(state);
        self.event.wait_clear();
        if !self.is_connected() {
            return Ok(());
        }

        // Keep reset from changing the generation between this check and the
        // enqueue, which could otherwise leak stale work into a new connection.
        let state = self.state.lock();
        if state.sync_generation != generation {
            return Ok(());
        }
        self.event.clear();
        self.send_all(to_send);
        Ok(())
    }

    fn send_or_buffer_update(
        &self,
        _lock: MutexGuard<'_, ()>,
        index: u32,
        rect: [usize; 4],
        to_send: &mut VecDeque<(FastVec<32>, bool)>,
        force: bool,
        generation: u64,
    ) -> Result<(), String> {
        let mut state = self.state.lock();
        // A reset after the producer checked `sync_required` belongs to a
        // later connection, so this work is stale.
        if state.sync_generation != generation {
            return Ok(());
        }
        match state.buffer {
            Buffer::Update(ref mut saved_index, ref mut saved_rect, ref mut data) => {
                if data.is_empty() {
                    *saved_index = index;
                    *saved_rect = rect;
                    if self.event.is_set() {
                        self.event.clear();
                        if let Some((message, send_now)) = to_send.pop_front() {
                            self.sender.send_set(message, send_now);
                        }
                        data.extend(to_send.drain(..));
                    } else {
                        data.extend(to_send.drain(..));
                    }
                    return Ok(());
                }

                if force && *saved_index == index && *saved_rect == rect {
                    data.clear();
                    data.extend(to_send.drain(..));
                    return Ok(());
                }
            }
            Buffer::Set(ref data) if data.is_empty() => {
                if self.event.is_set() {
                    self.event.clear();
                    if let Some((message, send_now)) = to_send.pop_front() {
                        self.sender.send_set(message, send_now);
                    }
                }
                state.buffer = Buffer::Update(index, rect, std::mem::take(to_send));
                return Ok(());
            }
            Buffer::Set(_) => {}
        }

        drop(state);
        self.event.wait_clear();
        if !self.is_connected() {
            return Ok(());
        }

        let mut state = self.state.lock();
        if state.sync_generation != generation {
            return Ok(());
        }
        self.event.clear();
        if let Some((message, send_now)) = to_send.pop_front() {
            self.sender.send_set(message, send_now);
        }
        state.buffer = Buffer::Update(index, rect, std::mem::take(to_send));
        Ok(())
    }

    fn send_control(&self, _lock: MutexGuard<'_, ()>, message: FastVec<32>, generation: u64) {
        self.event.wait();
        if !self.is_connected() {
            return;
        }

        let state = self.state.lock();
        if state.sync_generation != generation {
            return;
        }
        self.sender.send(message);
    }

    fn begin_sync(&self, messages: Vec<(FastVec<32>, bool)>, ack_count: usize) {
        let mut state = self.state.lock();
        state.buffer = Buffer::Set(Vec::new());
        state.sync_acks = ack_count;
        if ack_count == 0 {
            self.event.set();
        } else {
            self.event.clear();
        }
        drop(state);
        self.send_all(messages);
    }

    fn acknowledge(&self) {
        let mut state = self.state.lock();
        if state.sync_acks > 0 {
            state.sync_acks -= 1;
            if state.sync_acks > 0 {
                return;
            }
        }

        match state.buffer {
            Buffer::Set(ref mut data) => {
                if data.is_empty() {
                    self.event.set();
                } else {
                    let messages = std::mem::take(data);
                    drop(state);
                    self.send_all(messages);
                }
            }
            Buffer::Update(_, _, ref mut data) => match data.pop_front() {
                Some((message, send_now)) => self.sender.send_set(message, send_now),
                None => self.event.set(),
            },
        }
    }

    fn reset(&self) {
        self.event.set();
        let mut state = self.state.lock();
        state.buffer = Buffer::Set(Vec::new());
        state.sync_acks = 0;
        state.sync_generation = state.sync_generation.wrapping_add(1);
    }

    fn send_all(&self, messages: impl IntoIterator<Item = (FastVec<32>, bool)>) {
        for (message, send_now) in messages {
            self.sender.send_set(message, send_now);
        }
    }
}

struct StoredImage {
    data: Vec<u8>,
    size: [usize; 2],
}

struct ImageMultiInner {
    images: NoHashMap<u32, StoredImage>,
    sync_required: bool,
}

pub(crate) struct ImageMulti {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    inner: RwLock<ImageMultiInner>,
    transfer: ImageMultiTransfer,
}

impl ImageMulti {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            inner: RwLock::new(ImageMultiInner {
                images: NoHashMap::default(),
                sync_required: true,
            }),
            transfer: ImageMultiTransfer::new(id, sender, connected),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.inner.read().images.len()
    }

    pub(crate) fn contains(&self, index: u32) -> bool {
        self.inner.read().images.contains_key(&index)
    }

    pub(crate) fn indices(&self) -> Vec<u32> {
        let mut indices = self.inner.read().images.keys().copied().collect::<Vec<_>>();
        indices.sort_unstable();
        indices
    }

    pub(crate) fn get_size(&self, index: u32) -> Option<[usize; 2]> {
        self.inner.read().images.get(&index).map(|image| image.size)
    }

    pub(crate) fn get_image<R>(
        &self,
        index: u32,
        getter: impl FnOnce(Option<(&Vec<u8>, &[usize; 2])>) -> R,
    ) -> R {
        let inner = self.inner.read();
        getter(
            inner
                .images
                .get(&index)
                .map(|image| (&image.data, &image.size)),
        )
    }

    pub(crate) fn set_image(
        &self,
        index: u32,
        image: ImageData,
        update: bool,
    ) -> Result<(), String> {
        checked_fill_size(image.size)?;
        let to_send = if self.transfer.is_connected() {
            Some(pack_set_data(self.transfer.id, index, &image, update)?)
        } else {
            None
        };
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        let stored = inner.images.entry(index).or_insert_with(|| StoredImage {
            data: Vec::new(),
            size: image.size,
        });
        let resized = stored.size != image.size;
        stored.size = image.size;
        store_rgba(&mut stored.data, &image, resized);
        if inner.sync_required || !self.transfer.is_connected() {
            return Ok(());
        }
        let to_send = match to_send {
            Some(messages) => messages,
            None => pack_set_data(self.transfer.id, index, &image, update)?,
        };
        let generation = self.transfer.generation();
        drop(inner);
        self.transfer
            .send_or_buffer_set(lock, index, to_send, generation)
    }

    pub(crate) fn set_all_image(
        &self,
        index: u32,
        size: [usize; 2],
        rgba: [u8; 4],
        update: bool,
    ) -> Result<(), String> {
        let (wire_size, rgba_size) = checked_fill_size(size)?;
        let to_send = if self.transfer.is_connected() {
            Some(pack_fill_data(
                self.transfer.id,
                index,
                wire_size,
                rgba,
                update,
            )?)
        } else {
            None
        };
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        let stored = inner.images.entry(index).or_insert_with(|| StoredImage {
            data: Vec::new(),
            size,
        });
        stored.size = size;
        stored.data.clear();
        stored.data.reserve(rgba_size);
        let ptr = stored.data.as_mut_ptr().cast::<[u8; 4]>();
        unsafe {
            for pixel in 0..(size[0] * size[1]) {
                ptr.add(pixel).write(rgba);
            }
            stored.data.set_len(rgba_size);
        }
        if inner.sync_required || !self.transfer.is_connected() {
            return Ok(());
        }
        let to_send = match to_send {
            Some(messages) => messages,
            None => pack_fill_data(self.transfer.id, index, wire_size, rgba, update)?,
        };
        let generation = self.transfer.generation();
        drop(inner);
        self.transfer
            .send_or_buffer_set(lock, index, to_send, generation)
    }

    pub(crate) fn update_image(
        &self,
        index: u32,
        origin: &[usize; 2],
        image: ImageData,
        update: bool,
        force: bool,
    ) -> Result<(), CoreError> {
        checked_fill_size(image.size)?;
        checked_image_rect(origin, image.size)?;
        let to_send = if self.transfer.is_connected() {
            Some(pack_update_data(
                self.transfer.id,
                index,
                origin,
                &image,
                update,
            )?)
        } else {
            None
        };
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        let sync_required = inner.sync_required;
        let stored = inner
            .images
            .get_mut(&index)
            .ok_or_else(|| CoreError::not_found("ImageMulti index not found."))?;
        let end_row = origin[0].checked_add(image.size[0]);
        let end_column = origin[1].checked_add(image.size[1]);
        if end_row.is_none_or(|end| end > stored.size[0])
            || end_column.is_none_or(|end| end > stored.size[1])
        {
            return Err(format!("ImageMulti index {} update exceeds bounds", index).into());
        }
        let width = stored.size[1];
        write_rectangle(&image, &mut stored.data, width, origin);
        if sync_required || !self.transfer.is_connected() {
            return Ok(());
        }
        let mut to_send = match to_send {
            Some(messages) => messages,
            None => pack_update_data(self.transfer.id, index, origin, &image, update)?,
        };
        let rect = [origin[0], origin[1], image.size[0], image.size[1]];
        let generation = self.transfer.generation();
        drop(inner);
        self.transfer
            .send_or_buffer_update(lock, index, rect, &mut to_send, force, generation)?;
        Ok(())
    }

    pub(crate) fn remove_index(&self, index: u32, update: bool) -> Result<(), String> {
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        if inner.images.remove(&index).is_none() {
            return Ok(());
        }
        if inner.sync_required || !self.transfer.is_connected() {
            return Ok(());
        }
        let generation = self.transfer.generation();
        drop(inner);
        let message = ImageMultiHeader::Remove(index, update)
            .serialize(self.transfer.id)
            .map_err(|_| "Failed to serialize ImageMulti remove header".to_string())?;
        self.transfer.send_control(lock, message, generation);
        Ok(())
    }

    pub(crate) fn reset_images(&self, update: bool) -> Result<(), String> {
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        if inner.images.is_empty() {
            return Ok(());
        }
        inner.images.clear();
        if inner.sync_required || !self.transfer.is_connected() {
            return Ok(());
        }
        let generation = self.transfer.generation();
        drop(inner);
        let message = ImageMultiHeader::Reset(update)
            .serialize(self.transfer.id)
            .map_err(|_| "Failed to serialize ImageMulti reset header".to_string())?;
        self.transfer.send_control(lock, message, generation);
        Ok(())
    }
}

impl Acknowledge for ImageMulti {
    fn acknowledge(&self) {
        self.transfer.acknowledge();
    }

    fn reset(&self) {
        // Under the storage lock, so a producer either checked `sync_required`
        // and read the generation before this reset, or sees the new state.
        let mut inner = self.inner.write();
        inner.sync_required = true;
        self.transfer.reset();
    }
}

impl SyncTrait for ImageMulti {
    fn sync(&self) -> Result<(), ()> {
        let _lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        if !inner.sync_required {
            return Ok(());
        }

        let reset = ImageMultiHeader::Reset(false)
            .serialize(self.transfer.id)
            .map_err(|_| ())?;
        self.transfer.sender.send(reset);

        let mut indices = inner.images.keys().copied().collect::<Vec<_>>();
        indices.sort_unstable();
        let mut messages = Vec::new();
        for index in indices {
            let image = inner.images.get(&index).ok_or(())?;
            let image_data = ImageData::packed(&image.data, image.size, ImageType::ColorAlpha);
            messages.extend(
                pack_set_data(self.transfer.id, index, &image_data, false).map_err(|_| ())?,
            );
        }
        let ack_count = inner.images.len();
        inner.sync_required = false;
        self.transfer.begin_sync(messages, ack_count);
        Ok(())
    }
}

fn pack_fill_data(
    id: u64,
    index: u32,
    size: [u32; 2],
    rgba: [u8; 4],
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    checked_image_shape([size[0] as usize, size[1] as usize])?;
    let message = serialize_image_multi_header(id, index, ImageHeader::Fill(size, rgba, update), 0)
        .map_err(|_| format!("Failed to serialize fill header for image {}", id))?;
    Ok(vec![(message, false)])
}

fn pack_set_data(
    id: u64,
    index: u32,
    image: &ImageData,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_set_data_chunked(id, index, image, update, TRANSFER_CHUNK_SIZE)
}

pub(in crate::server_core) fn pack_set_data_chunked(
    id: u64,
    index: u32,
    image: &ImageData,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let size = checked_image_size(image.size)?;
    checked_image_bytes(image.size, 4)?;
    pack_image_set(id, image, size, update, chunk_limit, |header, len| {
        header.serialize_multi(id, index, image.image_type, len)
    })
}

fn pack_update_data(
    id: u64,
    index: u32,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    pack_update_data_chunked(id, index, origin, image, update, MSG_SIZE_THRESHOLD)
}

pub(in crate::server_core) fn pack_update_data_chunked(
    id: u64,
    index: u32,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
    chunk_limit: usize,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    let wire_rect = checked_image_rect(origin, image.size)?;
    checked_image_bytes(image.size, 4)?;
    pack_image_update(id, image, wire_rect, update, chunk_limit, |header, len| {
        serialize_image_multi_header(id, index, header, len)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc::error::TryRecvError;

    use super::*;
    #[cfg(feature = "client")]
    use crate::protocol::ServerHeader;
    #[cfg(feature = "client")]
    use crate::protocol::image::{ImageHeader, ImageSetHeader};
    use crate::server_core::net::sender::MessageReceiver;

    fn new_image_multi(is_connected: bool) -> (Arc<ImageMulti>, Arc<AtomicBool>, MessageReceiver) {
        let connected = Arc::new(AtomicBool::new(is_connected));
        let (sender, receiver) = MessageSender::new();
        let images = ImageMulti::new("images".to_string(), 1, sender, connected.clone());
        (images, connected, receiver)
    }

    fn image_data(data: &[u8], size: [usize; 2], image_type: ImageType) -> ImageData<'_> {
        ImageData::packed(data, size, image_type)
    }

    fn assert_message(receiver: &mut MessageReceiver) {
        assert!(matches!(receiver.try_recv(), Ok(Some(_))));
    }

    fn assert_no_message(receiver: &mut MessageReceiver) {
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn oversized_images_never_mutate_or_queue_even_when_disconnected() {
        for connected in [false, true] {
            let (images, _, mut receiver) = new_image_multi(connected);
            images.set_all_image(7, [1, 1], [9; 4], false).unwrap();
            for size in [[65_537, 1], [1, 65_537]] {
                let data = || image_data(&[], size, ImageType::Gray);
                for result in [
                    images.set_image(7, data(), false),
                    images.set_image(9, data(), false),
                    images.set_all_image(7, size, [0; 4], false),
                    images
                        .update_image(7, &[0, 0], data(), false, false)
                        .map_err(String::from),
                    pack_set_data(1, 7, &data(), false).map(|_| ()),
                    pack_fill_data(1, 7, [size[0] as u32, size[1] as u32], [0; 4], false)
                        .map(|_| ()),
                    pack_update_data(1, 7, &[0, 0], &data(), false).map(|_| ()),
                ] {
                    assert!(result.unwrap_err().contains("65536"));
                }
            }
            for origin in [[65_536, 0], [0, 65_536], [usize::MAX, 0]] {
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(images.update_image(7, &origin, data, false, false).is_err());
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(pack_update_data(1, 7, &origin, &data, false).is_err());
            }
            assert_eq!(
                images.get_image(7, |image| image.map(|(data, size)| (data.clone(), *size))),
                Some((vec![9; 4], [1, 1]))
            );
            assert_eq!(images.indices(), vec![7]);
            assert!(
                matches!(&images.transfer.state.lock().buffer, Buffer::Set(data) if data.is_empty())
            );
            assert_no_message(&mut receiver);
        }
    }

    fn marker_message(marker: u8) -> FastVec<32> {
        let mut message = FastVec::new();
        message.extend_from_slice(&[marker]);
        message
    }

    fn assert_marker(receiver: &mut MessageReceiver, expected: u8) {
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        assert_eq!(message.into_bytes().as_ref(), &[expected]);
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !predicate() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn same_sized_replacements_reuse_the_rgba_allocation() {
        let (images, _, _) = new_image_multi(false);
        let first = [1, 2, 3, 4, 5, 6, 7, 8];
        images
            .set_image(4, image_data(&first, [1, 2], ImageType::ColorAlpha), false)
            .unwrap();
        // Pointer stability is a limited allocation regression, not an allocation counter.
        let allocation = images.inner.read().images[&4].data.as_ptr();

        let second = [8, 7, 6, 5, 4, 3, 2, 1];
        images
            .set_image(4, image_data(&second, [1, 2], ImageType::ColorAlpha), false)
            .unwrap();
        assert_eq!(images.inner.read().images[&4].data.as_ptr(), allocation);
        assert_eq!(images.get_image(4, |v| v.unwrap().0.clone()), second);

        images
            .set_all_image(4, [1, 2], [9, 8, 7, 6], false)
            .unwrap();
        assert_eq!(images.inner.read().images[&4].data.as_ptr(), allocation);
        assert_eq!(
            images.get_image(4, |v| v.unwrap().0.clone()),
            [9, 8, 7, 6].repeat(2)
        );
    }

    #[test]
    fn invalid_dimensions_do_not_mutate_or_send() {
        let (images, _, mut receiver) = new_image_multi(true);
        let empty = [];
        assert!(
            images
                .set_image(1, image_data(&empty, [0, 1], ImageType::Gray), false)
                .is_err()
        );
        assert!(!images.contains(1));
        assert_no_message(&mut receiver);

        #[cfg(target_pointer_width = "64")]
        {
            let too_large = u32::MAX as usize + 1;
            assert!(
                images
                    .set_image(
                        1,
                        image_data(&empty, [1, too_large], ImageType::Gray),
                        false,
                    )
                    .unwrap_err()
                    .contains("65536")
            );
            assert!(!images.contains(1));
            assert_no_message(&mut receiver);
        }

        images
            .set_all_image(1, [1, 1], [1, 2, 3, 4], false)
            .unwrap();
        let before = images.get_image(1, |image| image.unwrap().0.clone());
        assert!(
            images
                .update_image(
                    1,
                    &[0, 0],
                    image_data(&empty, [1, 0], ImageType::Gray),
                    false,
                    false,
                )
                .is_err()
        );
        assert_eq!(
            images.get_image(1, |image| image.unwrap().0.clone()),
            before
        );
        assert_no_message(&mut receiver);
    }

    #[test]
    #[cfg(feature = "client")]
    fn sync_resets_sorts_and_waits_for_every_key_ack() {
        let (images, connected, mut receiver) = new_image_multi(false);
        images
            .set_all_image(7, [1, 1], [7, 7, 7, 255], false)
            .unwrap();
        images
            .set_all_image(2, [1, 1], [2, 2, 2, 255], false)
            .unwrap();
        assert_eq!(images.indices(), vec![2, 7]);

        connected.store(true, Ordering::Release);
        images.sync().unwrap();

        let (reset, _) = receiver.try_recv().unwrap().unwrap();
        let (reset, _) = ServerHeader::deserialize(&reset.into_bytes()).unwrap();
        assert!(matches!(
            reset,
            ServerHeader::ImageMulti(1, ImageMultiHeader::Reset(false))
        ));

        for expected_index in [2, 7] {
            let (message, _) = receiver.try_recv().unwrap().unwrap();
            let bytes = message.into_bytes();
            let (header, header_size) = ServerHeader::deserialize(&bytes).unwrap();
            match header {
                ServerHeader::ImageMulti(
                    1,
                    ImageMultiHeader::Modify(
                        index,
                        ImageHeader::Set(ImageSetHeader::All([1, 1], false), ImageType::ColorAlpha),
                        4,
                    ),
                ) => assert_eq!(index, expected_index),
                _ => panic!("unexpected ImageMulti sync header"),
            }
            assert_eq!(
                &bytes[header_size..],
                &[
                    expected_index as u8,
                    expected_index as u8,
                    expected_index as u8,
                    255
                ]
            );
        }
        assert_no_message(&mut receiver);
        assert!(!images.transfer.is_idle());

        images
            .set_all_image(9, [1, 1], [9, 9, 9, 255], true)
            .unwrap();
        assert_no_message(&mut receiver);
        images.acknowledge();
        assert_no_message(&mut receiver);
        images.acknowledge();

        let (message, _) = receiver.try_recv().unwrap().unwrap();
        let (header, _) = ServerHeader::deserialize(&message.into_bytes()).unwrap();
        assert!(matches!(
            header,
            ServerHeader::ImageMulti(
                1,
                ImageMultiHeader::Modify(9, ImageHeader::Fill([1, 1], [9, 9, 9, 255], true), 0)
            )
        ));
        images.acknowledge();
        assert!(images.transfer.is_idle());
    }

    #[test]
    #[cfg(feature = "client")]
    fn reconnect_sync_reproduces_only_current_keys() {
        let (images, connected, mut receiver) = new_image_multi(false);
        for index in [8, 3, 5] {
            images
                .set_all_image(index, [1, 1], [index as u8; 4], false)
                .unwrap();
        }

        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        for _ in 0..3 {
            assert_message(&mut receiver);
            images.acknowledge();
        }

        connected.store(false, Ordering::Release);
        images.reset();
        images.remove_index(5, false).unwrap();
        images.reset_images(false).unwrap();
        images.set_all_image(13, [1, 1], [13; 4], false).unwrap();
        images.set_all_image(1, [1, 1], [1; 4], false).unwrap();

        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        for expected_index in [1, 13] {
            let (message, _) = receiver.try_recv().unwrap().unwrap();
            let (header, _) = ServerHeader::deserialize(&message.into_bytes()).unwrap();
            match header {
                ServerHeader::ImageMulti(
                    1,
                    ImageMultiHeader::Modify(index, ImageHeader::Set(_, _), _),
                ) => assert_eq!(index, expected_index),
                _ => panic!("unexpected ImageMulti reconnect header"),
            }
        }
        assert_no_message(&mut receiver);
        assert_eq!(images.indices(), vec![1, 13]);
    }

    /// A producer that checked `sync_required` before a reset must not send
    /// its work into the next connection, on the idle fast path as well.
    #[test]
    fn work_checked_before_a_reset_is_never_sent() {
        let (images, _, mut receiver) = new_image_multi(true);
        let stale = images.transfer.generation();
        images.reset();
        images.sync().unwrap();
        assert_message(&mut receiver); // reconnect reset
        assert_no_message(&mut receiver);
        assert!(images.transfer.is_idle());

        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_set(lock, 7, vec![(marker_message(1), false)], stale)
            .unwrap();
        let lock = images.transfer.lock.lock();
        let mut update = VecDeque::from([(marker_message(2), false)]);
        images
            .transfer
            .send_or_buffer_update(lock, 7, [0, 0, 1, 1], &mut update, false, stale)
            .unwrap();
        let lock = images.transfer.lock.lock();
        images.transfer.send_control(lock, marker_message(3), stale);
        assert_no_message(&mut receiver);
        assert!(
            images.transfer.is_idle(),
            "stale work consumed the idle slot"
        );

        // Work of the current connection still goes out at once.
        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_set(
                lock,
                7,
                vec![(marker_message(4), false)],
                images.transfer.generation(),
            )
            .unwrap();
        assert_marker(&mut receiver, 4);
    }

    #[test]
    fn same_key_set_still_supersedes_a_buffered_update() {
        let (images, _, mut receiver) = new_image_multi(true);
        let mut update = VecDeque::from([(marker_message(1), true), (marker_message(2), false)]);
        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_update(
                lock,
                7,
                [0, 0, 1, 2],
                &mut update,
                false,
                images.transfer.generation(),
            )
            .unwrap();
        assert_marker(&mut receiver, 1);

        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_set(
                lock,
                7,
                vec![(marker_message(3), false)],
                images.transfer.generation(),
            )
            .unwrap();
        assert_no_message(&mut receiver);

        images.acknowledge();
        assert_marker(&mut receiver, 3);
        assert_no_message(&mut receiver);
        images.acknowledge();
        assert!(images.transfer.is_idle());
    }

    #[test]
    fn cross_key_set_waits_for_every_buffered_update_chunk() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, _, mut receiver) = new_image_multi(true);
        let mut update = VecDeque::from([(marker_message(1), true), (marker_message(2), false)]);
        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_update(
                lock,
                7,
                [0, 0, 1, 2],
                &mut update,
                false,
                images.transfer.generation(),
            )
            .unwrap();
        assert_marker(&mut receiver, 1);

        let waiting_images = images.clone();
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let waiting_set = std::thread::spawn(move || {
            let lock = waiting_images.transfer.lock.lock();
            started_sender.send(()).unwrap();
            let result = waiting_images.transfer.send_or_buffer_set(
                lock,
                9,
                vec![(marker_message(3), false)],
                waiting_images.transfer.generation(),
            );
            done_sender.send(result).unwrap();
        });

        started_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            done_receiver.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert_no_message(&mut receiver);

        images.acknowledge();
        assert_marker(&mut receiver, 2);
        assert!(matches!(
            done_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        images.acknowledge();
        done_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        waiting_set.join().unwrap();
        assert_marker(&mut receiver, 3);
        images.acknowledge();
        assert!(images.transfer.is_idle());
    }

    #[test]
    fn control_waits_for_every_buffered_update_chunk_without_consuming_idle() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, _, mut receiver) = new_image_multi(true);
        let mut update = VecDeque::from([(marker_message(1), true), (marker_message(2), false)]);
        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_update(
                lock,
                7,
                [0, 0, 1, 2],
                &mut update,
                false,
                images.transfer.generation(),
            )
            .unwrap();
        assert_marker(&mut receiver, 1);

        let waiting_images = images.clone();
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let waiting_control = std::thread::spawn(move || {
            let lock = waiting_images.transfer.lock.lock();
            started_sender.send(()).unwrap();
            waiting_images.transfer.send_control(
                lock,
                marker_message(3),
                waiting_images.transfer.generation(),
            );
            done_sender.send(()).unwrap();
        });

        started_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            done_receiver.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));

        images.acknowledge();
        assert_marker(&mut receiver, 2);
        assert!(matches!(
            done_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        images.acknowledge();
        done_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        waiting_control.join().unwrap();
        assert_marker(&mut receiver, 3);
        assert!(images.transfer.is_idle());

        let lock = images.transfer.lock.lock();
        images
            .transfer
            .send_or_buffer_set(
                lock,
                9,
                vec![(marker_message(4), false)],
                images.transfer.generation(),
            )
            .unwrap();
        assert_marker(&mut receiver, 4);
        images.acknowledge();
        assert!(images.transfer.is_idle());
    }

    #[test]
    fn remove_and_reset_controls_leave_transfer_ready_for_later_sets() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        images
            .set_all_image(1, [1, 1], [1, 1, 1, 255], false)
            .unwrap();
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        assert_message(&mut receiver); // initial image
        images.acknowledge();

        images
            .set_all_image(1, [1, 1], [2, 2, 2, 255], false)
            .unwrap();
        assert_message(&mut receiver);
        let removing_images = images.clone();
        let remove = std::thread::spawn(move || removing_images.remove_index(1, false));
        wait_until(|| !images.contains(1));
        assert_no_message(&mut receiver);
        images.acknowledge();
        remove.join().unwrap().unwrap();
        assert_message(&mut receiver); // remove
        assert!(images.transfer.is_idle());

        images
            .set_all_image(2, [1, 1], [3, 3, 3, 255], false)
            .unwrap();
        assert_message(&mut receiver);
        let resetting_images = images.clone();
        let reset = std::thread::spawn(move || resetting_images.reset_images(false));
        wait_until(|| images.len() == 0);
        assert_no_message(&mut receiver);
        images.acknowledge();
        reset.join().unwrap().unwrap();
        assert_message(&mut receiver); // reset
        assert!(images.transfer.is_idle());

        images
            .set_all_image(3, [1, 1], [4, 4, 4, 255], false)
            .unwrap();
        assert_message(&mut receiver);
        images.acknowledge();
        assert!(images.transfer.is_idle());
    }

    #[test]
    fn reset_discards_a_set_waiter_from_the_previous_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        images
            .set_all_image(1, [1, 1], [1, 1, 1, 255], false)
            .unwrap();
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        assert_message(&mut receiver); // initial image

        images
            .set_all_image(1, [1, 1], [2, 2, 2, 255], false)
            .unwrap();
        assert_no_message(&mut receiver);

        let waiting_images = images.clone();
        let waiter = std::thread::spawn(move || {
            waiting_images.set_all_image(1, [1, 1], [3, 3, 3, 255], false)
        });
        wait_until(|| images.get_image(1, |image| image.unwrap().0.as_slice() == [3, 3, 3, 255]));

        connected.store(false, Ordering::Release);
        images.reset();
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        waiter.join().unwrap().unwrap();

        assert_message(&mut receiver); // reconnect reset
        assert_message(&mut receiver); // latest complete image
        assert_no_message(&mut receiver);
    }

    #[test]
    #[cfg(feature = "client")]
    fn force_is_key_scoped_and_cross_key_updates_stay_ordered() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        for index in [1, 2] {
            images
                .set_all_image(index, [1, 1], [0, 0, 0, 255], false)
                .unwrap();
        }
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        assert_message(&mut receiver);
        assert_message(&mut receiver);
        images.acknowledge();
        images.acknowledge();

        let first = [10];
        images
            .update_image(
                1,
                &[0, 0],
                image_data(&first, [1, 1], ImageType::Gray),
                false,
                false,
            )
            .unwrap();
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        let bytes = message.into_bytes();
        let (header, header_size) = ServerHeader::deserialize(&bytes).unwrap();
        assert!(matches!(
            header,
            ServerHeader::ImageMulti(
                1,
                ImageMultiHeader::Modify(1, ImageHeader::Update(_, ImageType::Gray, false), 1)
            )
        ));
        assert_eq!(&bytes[header_size..], &[10]);

        let pending = [20];
        images
            .update_image(
                1,
                &[0, 0],
                image_data(&pending, [1, 1], ImageType::Gray),
                false,
                false,
            )
            .unwrap();
        let replacement = [30];
        images
            .update_image(
                1,
                &[0, 0],
                image_data(&replacement, [1, 1], ImageType::Gray),
                false,
                true,
            )
            .unwrap();
        assert_no_message(&mut receiver);

        let waiting_images = images.clone();
        let cross_key = std::thread::spawn(move || {
            let update = [40];
            waiting_images.update_image(
                2,
                &[0, 0],
                image_data(&update, [1, 1], ImageType::Gray),
                false,
                true,
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while images.get_image(2, |image| image.unwrap().0.clone()) != [40, 40, 40, 255] {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_no_message(&mut receiver);

        images.acknowledge();
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        let bytes = message.into_bytes();
        let (_, header_size) = ServerHeader::deserialize(&bytes).unwrap();
        assert_eq!(&bytes[header_size..], &[30]);

        images.acknowledge();
        cross_key.join().unwrap().unwrap();
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        let bytes = message.into_bytes();
        let (header, header_size) = ServerHeader::deserialize(&bytes).unwrap();
        assert!(matches!(
            header,
            ServerHeader::ImageMulti(
                1,
                ImageMultiHeader::Modify(2, ImageHeader::Update(_, ImageType::Gray, false), 1)
            )
        ));
        assert_eq!(&bytes[header_size..], &[40]);
        images.acknowledge();
        assert!(images.transfer.is_idle());
    }
}
