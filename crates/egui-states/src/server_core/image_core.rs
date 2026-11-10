use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, MutexGuard, RwLock, RwLockWriteGuard};

use crate::event::Event;
use crate::protocol::ServerHeader;
use crate::protocol::image::{
    ImageHeader, ImageType, checked_image_bounds, checked_image_bytes, checked_image_shape,
};
use crate::protocol::{FastVec, MSG_SIZE_THRESHOLD, TRANSFER_CHUNK_SIZE};
pub(crate) use crate::server_core::image_core_common::ImageData;
use crate::server_core::image_core_common::{
    checked_fill_size, pack_image_set, pack_image_update, store_rgba, write_rectangle,
};
use crate::server_core::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};

enum Buffer {
    Set(Vec<(FastVec<32>, bool)>),
    Update([usize; 4], VecDeque<(FastVec<32>, bool)>),
}

struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
    buffer: Buffer,
    sync_required: bool,
    sync_generation: u64,
}

pub(crate) struct Image {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    image: RwLock<ImageDataInner>,
    lock: Mutex<()>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl Image {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let event = Event::new();
        event.set(); // initially set so the first send does not block

        Arc::new(Self {
            name,
            id,
            image: RwLock::new(ImageDataInner {
                data: Vec::with_capacity(0),
                size: [0, 0],
                buffer: Buffer::Set(Vec::new()),
                sync_required: true,
                sync_generation: 0,
            }),
            lock: Mutex::new(()),
            sender,
            connected,
            event,
        })
    }

    pub(crate) fn get_size(&self) -> [usize; 2] {
        self.image.read().size
    }

    pub(crate) fn get_image<T>(&self, getter: impl FnOnce((&Vec<u8>, &[usize; 2])) -> T) -> T {
        let w = self.image.read();
        getter((&w.data, &w.size))
    }

    pub(crate) fn set_image(&self, image: ImageData, update: bool) -> Result<(), String> {
        checked_fill_size(image.size)?;
        // Prepare data to send if connected before locking
        let to_send = if self.connected.load(Ordering::Acquire) {
            Some(pack_set_data(self.id, &image, update)?)
        } else {
            None
        };

        // this is main lock for set and update operation
        let lock = self.lock.lock();
        let mut w = self.image.write();

        // Only allocate new data if the size has changed.
        let resized = image.size != w.size;
        w.size = image.size;
        store_rgba(&mut w.data, &image, resized);

        // A connection sync sends the latest complete image. Do not enqueue a
        // separate mutation until that initial sync has completed.
        if w.sync_required || !self.connected.load(Ordering::Acquire) {
            return Ok(());
        }

        // Reuse data prepared before locking, or prepare it now if the client
        // connected while the local image was being updated.
        let to_send = match to_send {
            Some(data) => data,
            None => pack_set_data(self.id, &image, update)?,
        };

        self.send_or_buffer_set(lock, w, to_send)
    }

    pub(crate) fn set_all_image(
        &self,
        size: [usize; 2],
        rgba: [u8; 4],
        update: bool,
    ) -> Result<(), String> {
        let (wire_size, rgba_size) = checked_fill_size(size)?;

        // Prepare the compact message before locking when possible, just like
        // set_image prepares its normal set messages.
        let to_send = if self.connected.load(Ordering::Acquire) {
            Some(pack_fill_data(self.id, wire_size, rgba, update)?)
        } else {
            None
        };

        // Use the same producer lock and set scheduling as set_image. This
        // keeps fill ordered with ordinary sets and updates.
        let lock = self.lock.lock();
        let mut w = self.image.write();

        w.size = size;

        w.data.clear();
        w.data.reserve(rgba_size);

        let ptr = w.data.as_mut_ptr().cast::<[u8; 4]>();
        unsafe {
            for i in 0..(size[0] * size[1]) {
                ptr.add(i).write(rgba);
            }

            w.data.set_len(rgba_size);
        }

        // The first sync for a connection always sends the complete cached
        // RGBA image. A compact fill is only useful after sync has completed.
        if w.sync_required || !self.connected.load(Ordering::Acquire) {
            return Ok(());
        }

        let to_send = match to_send {
            Some(data) => data,
            None => pack_fill_data(self.id, wire_size, rgba, update)?,
        };

        self.send_or_buffer_set(lock, w, to_send)
    }

    fn send_or_buffer_set(
        &self,
        _lock: MutexGuard<'_, ()>,
        mut w: RwLockWriteGuard<'_, ImageDataInner>,
        to_send: Vec<(FastVec<32>, bool)>,
    ) -> Result<(), String> {
        if let Buffer::Set(ref mut dat) = w.buffer {
            if dat.is_empty() {
                if self.event.is_set() {
                    self.event.clear();
                    for (message, send_now) in to_send {
                        self.sender.send_set(message, send_now);
                    }
                    return Ok(());
                } else {
                    dat.extend(to_send);
                    return Ok(());
                }
            }
        } else {
            //rewrite any update buffer, set has always priority over update
            if self.event.is_set() {
                self.event.clear();
                for (message, send_now) in to_send {
                    self.sender.send_set(message, send_now);
                }
                return Ok(());
            } else {
                w.buffer = Buffer::Set(to_send);
                return Ok(());
            }
        }

        // Buffer::Set already contains a pending set. Drop the image guard, but
        // keep the producer mutex, so acknowledge() can drain it before this
        // call sends its data.
        let sync_generation = w.sync_generation;
        drop(w);

        self.event.wait_clear();
        if !self.connected.load(Ordering::Acquire) {
            return Ok(());
        }

        // A reset may have released this waiter for a later connection. Keep
        // the read guard through enqueueing so reset/sync cannot race this
        // generation check.
        let w = self.image.read();
        if w.sync_generation != sync_generation {
            return Ok(());
        }

        for (message, send_now) in to_send {
            self.sender.send_set(message, send_now);
        }

        Ok(())
    }

    pub(crate) fn update_image(
        &self,
        origin: &[usize; 2],
        image: ImageData,
        update: bool,
        force: bool,
    ) -> Result<(), String> {
        checked_fill_size(image.size)?;
        checked_image_bounds(*origin, image.size)?;
        // Prepare data to send if connected before locking
        let to_send = if self.connected.load(Ordering::Acquire) {
            Some(pack_update_data(self.id, origin, &image, update)?)
        } else {
            None
        };

        // this is main lock for set and update operation
        let _lock = self.lock.lock();
        let mut w = self.image.write();

        let end_row = origin[0].checked_add(image.size[0]);
        let end_column = origin[1].checked_add(image.size[1]);
        if end_row.is_none_or(|end| end > w.size[0]) || end_column.is_none_or(|end| end > w.size[1])
        {
            return Err("Image size exceeds bounds".to_string());
        }

        // update local image data
        let width = w.size[1];
        write_rectangle(&image, &mut w.data, width, origin);

        // A connection sync sends the latest complete image. Do not enqueue a
        // separate mutation until that initial sync has completed.
        if w.sync_required || !self.connected.load(Ordering::Acquire) {
            return Ok(());
        }

        // Reuse data prepared before locking, or prepare it now if the client
        // connected while the local image was being updated.
        let mut to_send = match to_send {
            Some(data) => data,
            None => pack_update_data(self.id, origin, &image, update)?,
        };

        let new_rect = [origin[0], origin[1], image.size[0], image.size[1]];

        match w.buffer {
            Buffer::Update(ref mut rect, ref mut dat) => {
                if dat.is_empty() {
                    *rect = new_rect;
                    if self.event.is_set() {
                        self.event.clear();
                        if let Some((message, send_now)) = to_send.pop_front() {
                            self.sender.send_set(message, send_now);
                        }

                        if !to_send.is_empty() {
                            dat.extend(to_send);
                        }

                        return Ok(());
                    } else {
                        dat.extend(to_send);
                        return Ok(());
                    }
                }

                if force && new_rect == *rect {
                    dat.clear();
                    dat.extend(to_send);
                    return Ok(());
                }
            }
            Buffer::Set(ref dat) => {
                if dat.is_empty() {
                    if self.event.is_set() {
                        self.event.clear();
                        if let Some((message, send_now)) = to_send.pop_front() {
                            self.sender.send_set(message, send_now);
                        }
                        w.buffer = Buffer::Update(new_rect, to_send);

                        return Ok(());
                    } else {
                        w.buffer = Buffer::Update(new_rect, to_send);
                        return Ok(());
                    }
                }
            }
        }

        let sync_generation = w.sync_generation;
        drop(w);

        self.event.wait_clear();
        if !self.connected.load(Ordering::Acquire) {
            return Ok(());
        }

        let mut w = self.image.write();
        if w.sync_generation != sync_generation {
            return Ok(());
        }

        if let Some((message, send_now)) = to_send.pop_front() {
            self.sender.send_set(message, send_now);
        }
        w.buffer = Buffer::Update(new_rect, to_send);

        Ok(())
    }
}

impl Acknowledge for Image {
    fn acknowledge(&self) {
        let mut w = self.image.write();

        match w.buffer {
            Buffer::Set(ref mut dat) => {
                if dat.is_empty() {
                    self.event.set();
                    return;
                }

                for (message, send_now) in dat.drain(..) {
                    self.sender.send_set(message, send_now);
                }
            }
            Buffer::Update(_, ref mut dat) => match dat.pop_front() {
                Some((message, send_now)) => {
                    self.sender.send_set(message, send_now);
                }
                None => {
                    self.event.set();
                }
            },
        }
    }

    fn reset(&self) {
        self.event.set();
        let mut w = self.image.write();
        w.buffer = Buffer::Set(Vec::new());
        w.sync_required = true;
        w.sync_generation = w.sync_generation.wrapping_add(1);
    }
}

impl SyncTrait for Image {
    fn sync(&self) -> Result<(), ()> {
        // Serialize reconnect synchronization with producers. A stale producer
        // may hold this lock while waiting for reset() to set the event; taking
        // the lock here prevents sync from clearing that wake-up before the
        // producer consumes it and observes the new generation.
        let _lock = self.lock.lock();
        let mut w = self.image.write();
        if !w.sync_required {
            return Ok(());
        }

        if w.size[0] == 0 || w.size[1] == 0 {
            w.buffer = Buffer::Set(Vec::new());
            w.sync_required = false;
            self.event.set();
            return Ok(());
        }

        let image_data = ImageData::packed(&w.data, w.size, ImageType::ColorAlpha);

        let data = pack_set_data(self.id, &image_data, false).map_err(|_| ())?;

        w.buffer = Buffer::Set(Vec::new());
        w.sync_required = false;
        self.event.clear();
        for (message, send_now) in data {
            self.sender.send_set(message, send_now);
        }
        Ok(())
    }
}

fn pack_fill_data(
    id: u64,
    size: [u32; 2],
    rgba: [u8; 4],
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    checked_image_shape([size[0] as usize, size[1] as usize])?;
    let header = ServerHeader::Image(id, ImageHeader::Fill(size, rgba, update), 0);
    let message = crate::protocol::serialize_heap(&header)
        .map_err(|_| format!("Failed to serialize fill header for image {}", id))?;
    Ok(vec![(message, false)])
}

pub(crate) fn pack_set_data(
    id: u64,
    image: &ImageData,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    pack_set_data_chunked(id, image, update, TRANSFER_CHUNK_SIZE)
}

pub(super) fn pack_set_data_chunked(
    id: u64,
    image: &ImageData,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let [height, width] = checked_image_shape(image.size)?;
    checked_image_bytes(image.size, 4)?;
    pack_image_set(
        id,
        image,
        [width, height],
        update,
        chunk_limit,
        |header, len| header.serialize(id, image.image_type, len),
    )
}

fn pack_update_data(
    id: u64,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    pack_update_data_chunked(id, origin, image, update, MSG_SIZE_THRESHOLD)
}

pub(super) fn pack_update_data_chunked(
    id: u64,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
    chunk_limit: usize,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    checked_image_bounds(*origin, image.size)?;
    checked_image_bytes(image.size, 4)?;
    let [height, width] = checked_image_shape(image.size)?;
    let [y, x] = checked_image_shape(*origin)?;
    pack_image_update(
        id,
        image,
        [x, y, width, height],
        update,
        chunk_limit,
        |header, len| crate::protocol::serialize_heap(&ServerHeader::Image(id, header, len)),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc::error::TryRecvError;

    use super::*;
    use crate::server_core::sender::MessageReceiver;

    fn new_image(is_connected: bool) -> (Arc<Image>, Arc<AtomicBool>, MessageReceiver) {
        let connected = Arc::new(AtomicBool::new(is_connected));
        let (sender, receiver) = MessageSender::new();
        let image = Image::new("image".to_string(), 1, sender, connected.clone());
        (image, connected, receiver)
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
            let (image, _, mut receiver) = new_image(connected);
            image.set_all_image([1, 1], [9; 4], false).unwrap();
            for size in [[65_537, 1], [1, 65_537]] {
                let data = || image_data(&[], size, ImageType::Gray);
                for result in [
                    image.set_image(data(), false),
                    image.set_all_image(size, [0; 4], false),
                    image.update_image(&[0, 0], data(), false, false),
                    pack_set_data(1, &data(), false).map(|_| ()),
                    pack_fill_data(1, [size[0] as u32, size[1] as u32], [0; 4], false).map(|_| ()),
                    pack_update_data(1, &[0, 0], &data(), false).map(|_| ()),
                ] {
                    assert!(result.unwrap_err().contains("65536"));
                }
            }
            for origin in [[65_536, 0], [0, 65_536], [usize::MAX, 0]] {
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(image.update_image(&origin, data, false, false).is_err());
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(pack_update_data(1, &origin, &data, false).is_err());
            }
            assert_eq!(
                image.get_image(|(data, size)| (data.clone(), *size)),
                (vec![9; 4], [1, 1])
            );
            assert!(matches!(&image.image.read().buffer, Buffer::Set(data) if data.is_empty()));
            assert_no_message(&mut receiver);
        }
    }

    fn wait_for_image(image: &Image, expected: &[u8]) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while image.get_image(|(data, _)| data.clone()) != expected {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for image mutation"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn mutation_before_sync_is_sent_once_by_sync() {
        let (image, _, mut receiver) = new_image(true);
        let pixels = [10, 20, 30, 255];

        image
            .set_image(image_data(&pixels, [1, 1], ImageType::ColorAlpha), false)
            .unwrap();

        assert_no_message(&mut receiver);
        assert!(image.image.read().sync_required);

        image.sync().unwrap();

        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
        assert!(!image.image.read().sync_required);

        image.sync().unwrap();
        assert_no_message(&mut receiver);
    }

    #[test]
    fn mutation_after_sync_waits_for_sync_acknowledgement() {
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255];
        let replacement = [40, 50, 60, 255];

        image
            .set_image(image_data(&initial, [1, 1], ImageType::ColorAlpha), false)
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);

        image
            .set_image(
                image_data(&replacement, [1, 1], ImageType::ColorAlpha),
                false,
            )
            .unwrap();
        assert_no_message(&mut receiver);

        image.acknowledge();
        assert_message(&mut receiver);
        assert_no_message(&mut receiver);

        image.acknowledge();
        assert!(image.event.is_set());
    }

    #[test]
    fn update_before_sync_is_folded_into_full_image() {
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255, 40, 50, 60, 255];
        let update = [7, 8];

        image
            .set_image(image_data(&initial, [1, 2], ImageType::ColorAlpha), false)
            .unwrap();
        connected.store(true, Ordering::Release);
        image
            .update_image(
                &[0, 0],
                image_data(&update, [1, 1], ImageType::GrayAlpha),
                false,
                false,
            )
            .unwrap();

        assert_no_message(&mut receiver);
        assert_eq!(
            image.get_image(|(data, _)| data.clone()),
            vec![7, 7, 7, 8, 40, 50, 60, 255]
        );

        image.sync().unwrap();
        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
    }

    #[test]
    #[cfg(feature = "client")]
    fn fill_is_materialized_and_sent_without_pixel_data_after_sync() {
        let (image, connected, mut receiver) = new_image(false);

        image
            .set_all_image([2, 3], [10, 20, 30, 255], false)
            .unwrap();
        assert_eq!(image.get_size(), [2, 3]);
        assert_eq!(
            image.get_image(|(data, _)| data.clone()),
            [10, 20, 30, 255].repeat(6)
        );
        assert_no_message(&mut receiver);

        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        let (sync_message, _) = receiver.try_recv().unwrap().unwrap();
        let sync_bytes = sync_message.into_bytes();
        let (sync_header, sync_header_size) = ServerHeader::deserialize(&sync_bytes).unwrap();
        match sync_header {
            ServerHeader::Image(
                1,
                ImageHeader::Set(
                    crate::protocol::image::ImageSetHeader::All([3, 2], false),
                    ImageType::ColorAlpha,
                ),
                24,
            ) => {}
            _ => panic!("unexpected connection-sync header"),
        }
        assert_eq!(&sync_bytes[sync_header_size..], [10, 20, 30, 255].repeat(6));
        image.acknowledge();

        image.set_all_image([4, 5], [7, 7, 7, 8], true).unwrap();
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        let bytes = message.into_bytes();
        let (header, header_size) = ServerHeader::deserialize(&bytes).unwrap();
        assert_eq!(header_size, bytes.len());
        match header {
            ServerHeader::Image(1, ImageHeader::Fill([5, 4], [7, 7, 7, 8], true), 0) => {}
            _ => panic!("unexpected fill header"),
        }
        assert_eq!(
            image.get_image(|(data, _)| data.clone()),
            [7, 7, 7, 8].repeat(20)
        );
    }

    #[test]
    fn zero_sized_images_are_rejected_without_mutating_the_image() {
        for connected in [false, true] {
            let (image, _, mut receiver) = new_image(connected);
            image.set_all_image([1, 1], [9; 4], false).unwrap();
            for size in [[0, 5], [5, 0], [0, 0]] {
                let data = || image_data(&[], size, ImageType::Gray);
                assert!(image.set_image(data(), false).unwrap_err().contains("zero"));
                let result = image.update_image(&[0, 0], data(), false, false);
                assert!(result.unwrap_err().contains("zero"));
            }
            assert_eq!(
                image.get_image(|(data, size)| (data.clone(), *size)),
                (vec![9; 4], [1, 1])
            );
            assert_no_message(&mut receiver);
        }
    }

    #[test]
    fn fill_rejects_invalid_dimensions_without_mutating_the_image() {
        let (image, _, mut receiver) = new_image(false);

        image.set_all_image([1, 1], [9, 9, 9, 255], false).unwrap();
        assert!(image.set_all_image([0, 4], [0, 0, 0, 255], false).is_err());
        assert_eq!(image.get_size(), [1, 1]);
        assert_eq!(image.get_image(|(data, _)| data.clone()), [9, 9, 9, 255]);
        assert_no_message(&mut receiver);
    }

    #[test]
    fn reset_requires_one_new_connection_sync() {
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255];
        let replacement = [40, 50, 60, 255];

        image
            .set_image(image_data(&initial, [1, 1], ImageType::ColorAlpha), false)
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);
        image.acknowledge();

        connected.store(false, Ordering::Release);
        image.reset();
        assert!(image.image.read().sync_required);

        image
            .set_image(
                image_data(&replacement, [1, 1], ImageType::ColorAlpha),
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();

        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
        assert!(!image.image.read().sync_required);
    }

    #[test]
    fn reset_discards_a_waiter_from_the_previous_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255];
        let buffered = [40, 50, 60, 255];

        image
            .set_image(image_data(&initial, [1, 1], ImageType::ColorAlpha), false)
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);

        image
            .set_image(image_data(&buffered, [1, 1], ImageType::ColorAlpha), false)
            .unwrap();
        assert_no_message(&mut receiver);

        let waiting_image = image.clone();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let latest = [70, 80, 90, 255];
            let result =
                waiting_image.set_image(image_data(&latest, [1, 1], ImageType::ColorAlpha), false);
            done_sender.send(result).unwrap();
        });

        wait_for_image(&image, &[70, 80, 90, 255]);
        assert!(image.lock.try_lock().is_none());

        connected.store(false, Ordering::Release);
        image.reset();
        connected.store(true, Ordering::Release);
        let syncing_image = image.clone();
        let (sync_sender, sync_receiver) = std::sync::mpsc::channel();
        let syncer = std::thread::spawn(move || {
            sync_sender.send(syncing_image.sync()).unwrap();
        });
        done_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("stale set producer did not finish after reconnect reset")
            .unwrap();
        sync_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("image sync did not finish after stale set producer")
            .unwrap();
        waiter.join().unwrap();
        syncer.join().unwrap();

        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
    }

    #[test]
    fn reset_discards_an_update_waiter_from_the_previous_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255, 40, 50, 60, 255];
        let buffered = [15, 25, 35, 255, 45, 55, 65, 255];

        image
            .set_image(image_data(&initial, [1, 2], ImageType::ColorAlpha), false)
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);

        image
            .set_image(image_data(&buffered, [1, 2], ImageType::ColorAlpha), false)
            .unwrap();
        assert_no_message(&mut receiver);

        let waiting_image = image.clone();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let update = [70, 80];
            let result = waiting_image.update_image(
                &[0, 0],
                image_data(&update, [1, 1], ImageType::GrayAlpha),
                false,
                false,
            );
            done_sender.send(result).unwrap();
        });

        wait_for_image(&image, &[70, 70, 70, 80, 45, 55, 65, 255]);
        assert!(image.lock.try_lock().is_none());

        connected.store(false, Ordering::Release);
        image.reset();
        connected.store(true, Ordering::Release);
        let syncing_image = image.clone();
        let (sync_sender, sync_receiver) = std::sync::mpsc::channel();
        let syncer = std::thread::spawn(move || {
            sync_sender.send(syncing_image.sync()).unwrap();
        });
        done_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("stale update producer did not finish after reconnect reset")
            .unwrap();
        sync_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("image sync did not finish after stale update producer")
            .unwrap();
        waiter.join().unwrap();
        syncer.join().unwrap();

        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
    }
}

#[cfg(all(test, feature = "client"))]
mod packing_tests {
    use super::*;
    use crate::client::image::Image as ClientImage;
    use crate::client::messages::{MessageSender as ClientSender, MessagesParser};

    #[test]
    fn real_image_set_and_update_chunks_preserve_pixels_and_publication_boundaries() {
        let width = 1024;
        let threshold_rows = MSG_SIZE_THRESHOLD / (width * 4);
        for height in [
            threshold_rows - 1,
            threshold_rows,
            threshold_rows + 1,
            2 * threshold_rows + 3,
        ] {
            let context = egui::Context::default();
            let (sender, mut receiver) = ClientSender::new();
            let client = ClientImage::new("image".into(), 71, sender, context.clone());
            // A previous image must stay visible until the new set completes.
            client.fill_image([1, 1], [0; 4], &[]).unwrap();
            crate::test_support::ack(&mut receiver, 71);
            let _ = crate::test_support::texture_updates(&context);
            let mut expected: Vec<u8> = (0..height * width * 4)
                .map(|i| (i * 31 % 251) as u8)
                .collect();
            let shape = [height, width];
            let image = ImageData::packed(&expected, shape, ImageType::ColorAlpha);
            let packed = pack_set_data(71, &image, true).unwrap();
            let total = packed.len();
            assert_eq!(total, expected.len().div_ceil(TRANSFER_CHUNK_SIZE));
            for (n, (message, _)) in packed.into_iter().enumerate() {
                let (mut parser, (header, payload)) =
                    MessagesParser::from_bytes(message.into_bytes()).unwrap();
                assert_eq!(header.update_flag(), n + 1 == total);
                match header {
                    ServerHeader::Image(71, ImageHeader::Set(message, kind), _) => {
                        client.set_image(message, kind, &payload).unwrap();
                    }
                    _ => panic!("unexpected packed image set"),
                }
                assert!(parser.next().unwrap().is_none());
                if n + 1 < total {
                    assert_eq!(client.get_size(), Some([1, 1]));
                    assert!(receiver.try_recv().is_err());
                    assert!(
                        crate::test_support::texture_updates(&context).is_empty(),
                        "partial set was published"
                    );
                }
            }
            crate::test_support::ack(&mut receiver, 71);
            let delta = crate::test_support::texture_updates(&context);
            let changes = &delta[&client.get_id().unwrap()];
            assert_eq!(changes.len(), 1);
            let egui::ImageData::Color(pixels) = &changes[0].image;
            assert_eq!(pixels.size, [width, height]);
            assert_eq!(
                pixels
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>(),
                expected
            );
            drop(delta);

            // Update messages publish complete row strips individually. The last
            // strip requests repaint; every strip is acknowledged.
            expected.iter_mut().for_each(|v| *v = 255 - *v);
            let image = ImageData::packed(&expected, shape, ImageType::ColorAlpha);
            let packed = pack_update_data(71, &[0, 0], &image, true).unwrap();
            let total = packed.len();
            let mut reconstructed = vec![0; expected.len()];
            let mut next_row = 0;
            for (n, (message, _)) in packed.into_iter().enumerate() {
                let (_, (header, payload)) =
                    MessagesParser::from_bytes(message.into_bytes()).unwrap();
                assert_eq!(header.update_flag(), n + 1 == total);
                match header {
                    ServerHeader::Image(71, ImageHeader::Update(rect, kind, _), _) => {
                        assert_eq!(rect[0], 0);
                        assert_eq!(rect[1] as usize, next_row);
                        assert_eq!(rect[2] as usize, width);
                        client.update_image(rect, kind, &payload).unwrap();
                        next_row += rect[3] as usize;
                    }
                    _ => panic!("unexpected packed image update"),
                }
                crate::test_support::ack(&mut receiver, 71);
                let delta = crate::test_support::texture_updates(&context);
                let changes = &delta[&client.get_id().unwrap()];
                assert_eq!(changes.len(), 1);
                let patch = &changes[0];
                let egui::ImageData::Color(pixels) = &patch.image;
                let pos = patch.pos.unwrap_or_else(|| {
                    assert_eq!(
                        pixels.size,
                        [width, height],
                        "a full delta must cover the texture"
                    );
                    [0, 0]
                });
                let bytes: Vec<u8> = pixels.pixels.iter().flat_map(|p| p.to_array()).collect();
                let offset = pos[1] * width * 4;
                reconstructed[offset..offset + bytes.len()].copy_from_slice(&bytes);
            }
            assert_eq!(next_row, height);
            assert_eq!(reconstructed, expected);
            assert_eq!(client.get_size(), Some([width, height]));
        }
    }
}
