use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;

use crate::protocol::ServerHeader;
use crate::protocol::image::{
    ImageHeader, ImageType, checked_image_bounds, checked_image_bytes, checked_image_shape,
};
use crate::protocol::{FastVec, TRANSFER_CHUNK_SIZE};
use crate::server_core::net::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
pub(crate) use crate::server_core::states::image::common::ImageData;
use crate::server_core::states::image::common::{
    checked_fill_size, checked_max_lines, pack_image_set, pack_image_update, store_rgba,
    write_rectangle,
};
use crate::server_core::states::image::scheduler::Scheduler;
use crate::server_core::transfer::Messages;

/// The single key of an `Image` in its scheduler.
const KEY: u32 = 0;

struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
}

pub(crate) struct Image {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    image: RwLock<ImageDataInner>,
    scheduler: Scheduler,
}

impl Image {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            image: RwLock::new(ImageDataInner {
                data: Vec::with_capacity(0),
                size: [0, 0],
            }),
            scheduler: Scheduler::new(id, sender, connected),
        })
    }

    pub(crate) fn get_size(&self) -> [usize; 2] {
        self.image.read().size
    }

    pub(crate) fn get_image<T>(&self, getter: impl FnOnce((&Vec<u8>, &[usize; 2])) -> T) -> T {
        let w = self.image.read();
        getter((&w.data, &w.size))
    }

    /// Whether nothing is in flight and nothing waits.
    pub(crate) fn is_idle(&self) -> bool {
        self.scheduler.is_idle()
    }

    /// Replaces the image. Sends it once the state is idle, dropping waiting
    /// parts of a split update; with `wait`, returns once the client
    /// acknowledged it.
    pub(crate) fn set_image(
        &self,
        image: ImageData,
        update: bool,
        wait: bool,
    ) -> Result<(), String> {
        checked_fill_size(image.size)?;
        let id = self.scheduler.id;
        // Prepare data to send if connected before locking
        let to_send = if self.scheduler.is_connected() {
            Some(pack_set_data(id, &image, update)?)
        } else {
            None
        };

        // this is main lock for set and update operation
        let lock = self.scheduler.lock.lock();
        let mut w = self.image.write();

        // Only allocate new data if the size has changed.
        let resized = image.size != w.size;
        w.size = image.size;
        store_rgba(&mut w.data, &image, resized);

        // A connection sync sends the latest complete image. Do not enqueue a
        // separate mutation until that initial sync has completed.
        let Some(generation) = self.scheduler.sendable() else {
            return Ok(());
        };

        // Reuse data prepared before locking, or prepare it now if the client
        // connected while the local image was being updated.
        let to_send = match to_send {
            Some(data) => data,
            None => pack_set_data(id, &image, update)?,
        };
        drop(w);

        self.scheduler
            .send_set(lock, KEY, to_send, wait, generation);
        Ok(())
    }

    pub(crate) fn set_all_image(
        &self,
        size: [usize; 2],
        rgba: [u8; 4],
        update: bool,
        wait: bool,
    ) -> Result<(), String> {
        let (wire_size, rgba_size) = checked_fill_size(size)?;
        let id = self.scheduler.id;

        // Prepare the compact message before locking when possible, just like
        // set_image prepares its normal set messages.
        let to_send = if self.scheduler.is_connected() {
            Some(pack_fill_data(id, wire_size, rgba, update)?)
        } else {
            None
        };

        // Use the same producer lock and set scheduling as set_image. This
        // keeps fill ordered with ordinary sets and updates.
        let lock = self.scheduler.lock.lock();
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
        let Some(generation) = self.scheduler.sendable() else {
            return Ok(());
        };

        let to_send = match to_send {
            Some(data) => data,
            None => pack_fill_data(id, wire_size, rgba, update)?,
        };
        drop(w);

        self.scheduler
            .send_set(lock, KEY, to_send, wait, generation);
        Ok(())
    }

    /// Writes a rectangle and sends it once the state is idle, in parts of at
    /// most `max_lines` rows. With `force`, waiting parts of an earlier split
    /// update are dropped first when the new rectangle covers it. With
    /// `wait`, returns once the client acknowledged the last part.
    pub(crate) fn update_image(
        &self,
        origin: &[usize; 2],
        image: ImageData,
        update: bool,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<(), String> {
        checked_fill_size(image.size)?;
        checked_image_bounds(*origin, image.size)?;
        checked_max_lines(max_lines)?;
        let id = self.scheduler.id;
        // Prepare data to send if connected before locking
        let to_send = if self.scheduler.is_connected() {
            Some(pack_update_data(id, origin, &image, update, max_lines)?)
        } else {
            None
        };

        // this is main lock for set and update operation
        let lock = self.scheduler.lock.lock();
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
        let Some(generation) = self.scheduler.sendable() else {
            return Ok(());
        };

        // Reuse data prepared before locking, or prepare it now if the client
        // connected while the local image was being updated.
        let to_send = match to_send {
            Some(data) => data,
            None => pack_update_data(id, origin, &image, update, max_lines)?,
        };
        drop(w);

        let rect = [origin[0], origin[1], image.size[0], image.size[1]];
        self.scheduler
            .send_update(lock, KEY, rect, to_send, force, wait, generation);
        Ok(())
    }
}

impl Acknowledge for Image {
    fn acknowledge(&self) {
        self.scheduler.acknowledge();
    }

    fn reset(&self) {
        // Under the storage lock, so a producer either checked whether it may
        // send before this reset, or sees the new state.
        let _w = self.image.write();
        self.scheduler.reset();
    }
}

impl SyncTrait for Image {
    fn sync(&self) -> Result<(), ()> {
        // Serialize reconnect synchronization with producers. A stale producer
        // may hold this lock while waiting for reset() to set the event; taking
        // the lock here prevents sync from clearing that wake-up before the
        // producer consumes it and observes the new generation.
        let _lock = self.scheduler.lock.lock();
        let w = self.image.write();
        if !self.scheduler.required() {
            return Ok(());
        }

        if w.size[0] == 0 || w.size[1] == 0 {
            self.scheduler.begin_sync(Vec::new(), 0);
            return Ok(());
        }

        let image_data = ImageData::packed(&w.data, w.size, ImageType::ColorAlpha);
        let data = pack_set_data(self.scheduler.id, &image_data, false).map_err(|_| ())?;
        self.scheduler.begin_sync(data, 1);
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

pub(in crate::server_core) fn pack_set_data_chunked(
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
    max_lines: Option<usize>,
) -> Result<VecDeque<Messages>, String> {
    pack_update_data_chunked(id, origin, image, update, max_lines, TRANSFER_CHUNK_SIZE)
}

pub(in crate::server_core) fn pack_update_data_chunked(
    id: u64,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
    max_lines: Option<usize>,
    chunk_limit: usize,
) -> Result<VecDeque<Messages>, String> {
    checked_image_bounds(*origin, image.size)?;
    checked_image_bytes(image.size, 4)?;
    let [height, width] = checked_image_shape(image.size)?;
    let [y, x] = checked_image_shape(*origin)?;
    pack_image_update(
        id,
        image,
        [x, y, width, height],
        update,
        max_lines,
        chunk_limit,
        |header, len| crate::protocol::serialize_heap(&ServerHeader::Image(id, header, len)),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc::error::TryRecvError;

    use super::*;
    use crate::server_core::net::sender::MessageReceiver;

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
            image.set_all_image([1, 1], [9; 4], false, false).unwrap();
            for size in [[65_537, 1], [1, 65_537]] {
                let data = || image_data(&[], size, ImageType::Gray);
                for result in [
                    image.set_image(data(), false, false),
                    image.set_all_image(size, [0; 4], false, false),
                    image.update_image(&[0, 0], data(), false, false, None, false),
                    pack_set_data(1, &data(), false).map(|_| ()),
                    pack_fill_data(1, [size[0] as u32, size[1] as u32], [0; 4], false).map(|_| ()),
                    pack_update_data(1, &[0, 0], &data(), false, None).map(|_| ()),
                ] {
                    assert!(result.unwrap_err().contains("65536"));
                }
            }
            for origin in [[65_536, 0], [0, 65_536], [usize::MAX, 0]] {
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(
                    image
                        .update_image(&origin, data, false, false, None, false)
                        .is_err()
                );
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(pack_update_data(1, &origin, &data, false, None).is_err());
            }
            assert_eq!(
                image.get_image(|(data, size)| (data.clone(), *size)),
                (vec![9; 4], [1, 1])
            );
            assert!(image.is_idle());
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
            .set_image(
                image_data(&pixels, [1, 1], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();

        assert_no_message(&mut receiver);
        assert!(image.scheduler.required());

        image.sync().unwrap();

        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
        assert!(!image.scheduler.required());

        image.sync().unwrap();
        assert_no_message(&mut receiver);
    }

    #[test]
    fn mutation_after_sync_waits_for_sync_acknowledgement() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255];

        image
            .set_image(
                image_data(&initial, [1, 1], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);
        assert!(!image.is_idle());

        let done = spawn_blocked(&image, |image| {
            let replacement = [40, 50, 60, 255];
            image.set_image(
                image_data(&replacement, [1, 1], ImageType::ColorAlpha),
                false,
                false,
            )
        });
        assert_no_message(&mut receiver);

        image.acknowledge();
        finished(&done);
        assert_message(&mut receiver);
        assert_no_message(&mut receiver);

        image.acknowledge();
        assert!(image.is_idle());
    }

    #[test]
    fn update_before_sync_is_folded_into_full_image() {
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255, 40, 50, 60, 255];
        let update = [7, 8];

        image
            .set_image(
                image_data(&initial, [1, 2], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        image
            .update_image(
                &[0, 0],
                image_data(&update, [1, 1], ImageType::GrayAlpha),
                false,
                false,
                None,
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
            .set_all_image([2, 3], [10, 20, 30, 255], false, false)
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

        image
            .set_all_image([4, 5], [7, 7, 7, 8], true, false)
            .unwrap();
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
            image.set_all_image([1, 1], [9; 4], false, false).unwrap();
            for size in [[0, 5], [5, 0], [0, 0]] {
                let data = || image_data(&[], size, ImageType::Gray);
                assert!(
                    image
                        .set_image(data(), false, false)
                        .unwrap_err()
                        .contains("zero")
                );
                let result = image.update_image(&[0, 0], data(), false, false, None, false);
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

        image
            .set_all_image([1, 1], [9, 9, 9, 255], false, false)
            .unwrap();
        assert!(
            image
                .set_all_image([0, 4], [0, 0, 0, 255], false, false)
                .is_err()
        );
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
            .set_image(
                image_data(&initial, [1, 1], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);
        image.acknowledge();

        connected.store(false, Ordering::Release);
        image.reset();
        assert!(image.scheduler.required());

        image
            .set_image(
                image_data(&replacement, [1, 1], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();

        assert_message(&mut receiver);
        assert_no_message(&mut receiver);
        assert!(!image.scheduler.required());
    }

    /// A producer call run on another thread.
    type Producer = Box<dyn FnOnce(&Image) -> Result<(), String> + Send>;

    #[test]
    fn reset_discards_a_waiter_from_the_previous_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, connected, mut receiver) = new_image(false);
        let initial = [10, 20, 30, 255, 40, 50, 60, 255];

        image
            .set_image(
                image_data(&initial, [1, 2], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);

        // The synchronization is not acknowledged, so both producers wait.
        let set: Producer = Box::new(|image| {
            let latest = [70, 80, 90, 255, 70, 80, 90, 255];
            image.set_image(
                image_data(&latest, [1, 2], ImageType::ColorAlpha),
                false,
                false,
            )
        });
        let update: Producer = Box::new(|image| {
            let latest = [70, 80];
            image.update_image(
                &[0, 0],
                image_data(&latest, [1, 1], ImageType::GrayAlpha),
                false,
                false,
                None,
                false,
            )
        });
        let expected = [
            [70, 80, 90, 255, 70, 80, 90, 255],
            [70, 70, 70, 80, 70, 80, 90, 255],
        ];
        for (operation, expected) in [set, update].into_iter().zip(expected) {
            let waiting_image = image.clone();
            let (done_sender, done_receiver) = std::sync::mpsc::channel();
            let waiter = std::thread::spawn(move || {
                done_sender.send(operation(&waiting_image)).unwrap();
            });

            wait_for_image(&image, &expected);
            assert!(image.scheduler.lock.try_lock().is_none());

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
                .expect("stale producer did not finish after reconnect reset")
                .unwrap();
            sync_receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("image sync did not finish after stale producer")
                .unwrap();
            waiter.join().unwrap();
            syncer.join().unwrap();

            // Only the synchronization of the latest image is sent.
            assert_message(&mut receiver);
            assert_no_message(&mut receiver);
        }
    }

    fn bytes(message: &FastVec<32>) -> Vec<u8> {
        match message {
            FastVec::Heap(data) => data.clone(),
            FastVec::Stack(data) => data.as_ref().to_vec(),
        }
    }

    fn assert_sent(receiver: &mut MessageReceiver, expected: &Messages) {
        for (message, _) in expected {
            let (sent, _) = receiver.try_recv().unwrap().unwrap();
            assert_eq!(bytes(&sent), bytes(message));
        }
    }

    /// An image of four gray rows, synchronized and acknowledged.
    fn synced_image(rows: [u8; 4]) -> (Arc<Image>, MessageReceiver) {
        let (image, connected, mut receiver) = new_image(false);
        image
            .set_image(image_data(&rows, [4, 1], ImageType::Gray), false, false)
            .unwrap();
        connected.store(true, Ordering::Release);
        image.sync().unwrap();
        assert_message(&mut receiver);
        image.acknowledge();
        assert!(image.is_idle());
        (image, receiver)
    }

    /// Runs `operation` on another thread and returns a receiver for its
    /// result, after checking that it is still blocked.
    fn spawn_blocked(
        image: &Arc<Image>,
        operation: impl FnOnce(&Image) -> Result<(), String> + Send + 'static,
    ) -> std::sync::mpsc::Receiver<Result<(), String>> {
        let image = image.clone();
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || done_sender.send(operation(&image)).unwrap());
        assert!(matches!(
            done_receiver.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        done_receiver
    }

    fn finished(done: &std::sync::mpsc::Receiver<Result<(), String>>) {
        done.recv_timeout(std::time::Duration::from_secs(5))
            .expect("blocked image operation did not finish")
            .unwrap();
    }

    /// A set waits for the transfer in flight and drops the waiting parts of
    /// a split update, which it overwrites.
    #[test]
    fn a_set_drops_the_waiting_parts_of_a_split_update() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, mut receiver) = synced_image([0; 4]);
        let rows = [1, 2, 3, 4];
        let groups = pack_update_data(
            1,
            &[0, 0],
            &image_data(&rows, [4, 1], ImageType::Gray),
            false,
            Some(1),
        )
        .unwrap();
        image
            .update_image(
                &[0, 0],
                image_data(&rows, [4, 1], ImageType::Gray),
                false,
                false,
                Some(1),
                false,
            )
            .unwrap();
        assert_sent(&mut receiver, &groups[0]);

        let done = spawn_blocked(&image, |image| {
            image.set_image(image_data(&[5; 4], [4, 1], ImageType::Gray), false, false)
        });
        assert_no_message(&mut receiver);
        image.acknowledge();
        finished(&done);
        assert_sent(
            &mut receiver,
            &pack_set_data(1, &image_data(&[5; 4], [4, 1], ImageType::Gray), false).unwrap(),
        );
        assert_no_message(&mut receiver);
        image.acknowledge();
        assert!(image.is_idle());
        assert_eq!(
            image.get_image(|(data, _)| data.clone()),
            [5, 5, 5, 255].repeat(4)
        );
    }

    /// With `wait`, a set returns only once the client acknowledged it.
    #[test]
    fn a_set_with_wait_returns_once_it_is_acknowledged() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, mut receiver) = synced_image([0; 4]);
        let done = spawn_blocked(&image, |image| {
            image.set_image(image_data(&[6; 4], [4, 1], ImageType::Gray), false, true)
        });
        assert_message(&mut receiver);
        image.acknowledge();
        finished(&done);
        assert!(image.is_idle());
    }

    #[test]
    fn a_split_update_sends_each_part_after_an_ack() {
        let (image, mut receiver) = synced_image([0; 4]);
        let rows = [1, 2, 3, 4];
        let data = || image_data(&rows, [4, 1], ImageType::Gray);
        let groups = pack_update_data(1, &[0, 0], &data(), true, Some(2)).unwrap();
        assert_eq!(groups.len(), 2);
        image
            .update_image(&[0, 0], data(), true, false, Some(2), false)
            .unwrap();
        for group in groups {
            assert_sent(&mut receiver, &group);
            assert_no_message(&mut receiver);
            assert!(!image.is_idle());
            image.acknowledge();
        }
        assert_no_message(&mut receiver);
        assert!(image.is_idle());

        assert!(
            image
                .update_image(&[0, 0], data(), true, false, Some(0), false)
                .is_err()
        );
        assert_no_message(&mut receiver);
    }

    #[test]
    fn force_update_drops_waiting_parts_it_covers() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, mut receiver) = synced_image([0; 4]);
        let old = [1, 1, 1, 1];
        let new = [2, 2, 2, 2];
        let old_groups = pack_update_data(
            1,
            &[0, 0],
            &image_data(&old, [4, 1], ImageType::Gray),
            false,
            Some(1),
        )
        .unwrap();
        let new_groups = pack_update_data(
            1,
            &[0, 0],
            &image_data(&new, [4, 1], ImageType::Gray),
            false,
            Some(1),
        )
        .unwrap();
        image
            .update_image(
                &[0, 0],
                image_data(&old, [4, 1], ImageType::Gray),
                false,
                false,
                Some(1),
                false,
            )
            .unwrap();
        assert_sent(&mut receiver, &old_groups[0]);
        // Covers the old rectangle: its three waiting parts are dropped, and
        // the call waits only for the part in flight.
        let done = spawn_blocked(&image, move |image| {
            image.update_image(
                &[0, 0],
                image_data(&new, [4, 1], ImageType::Gray),
                false,
                true,
                Some(1),
                false,
            )
        });
        assert_no_message(&mut receiver);
        image.acknowledge();
        finished(&done);
        for group in new_groups {
            assert_sent(&mut receiver, &group);
            assert_no_message(&mut receiver);
            image.acknowledge();
        }
        assert!(image.is_idle());
        assert_eq!(
            image.get_image(|(data, _)| data.clone()),
            [2, 2, 2, 255].repeat(4)
        );
    }

    #[test]
    fn force_update_waits_for_work_it_does_not_cover() {
        if crate::test_support::isolated() {
            return;
        }
        let (image, mut receiver) = synced_image([0; 4]);
        let rows = [1, 2, 3, 4];
        let groups = pack_update_data(
            1,
            &[0, 0],
            &image_data(&rows, [4, 1], ImageType::Gray),
            false,
            Some(2),
        )
        .unwrap();
        image
            .update_image(
                &[0, 0],
                image_data(&rows, [4, 1], ImageType::Gray),
                false,
                false,
                Some(2),
                false,
            )
            .unwrap();
        assert_sent(&mut receiver, &groups[0]);

        // Rows 0..2 do not cover the waiting update's rows 0..4.
        let done = spawn_blocked(&image, |image| {
            image.update_image(
                &[0, 0],
                image_data(&[9, 9], [2, 1], ImageType::Gray),
                false,
                true,
                None,
                false,
            )
        });
        image.acknowledge();
        assert_sent(&mut receiver, &groups[1]);
        assert!(done.try_recv().is_err());
        image.acknowledge();
        finished(&done);
        assert_message(&mut receiver);
        image.acknowledge();
        assert!(image.is_idle());
    }
}

#[cfg(all(test, feature = "client"))]
mod packing_tests {
    use super::*;
    use crate::client::messages::{MessageSender as ClientSender, MessagesParser};
    use crate::client::states::image::Image as ClientImage;

    type ClientReceiver =
        tokio::sync::mpsc::UnboundedReceiver<Option<crate::client::messages::ChannelMessage>>;

    /// Feeds one group of update messages to `client`, checks that nothing is
    /// published or acknowledged before its last message, and returns the
    /// single texture change it made as its position and RGBA bytes.
    fn apply_update_group(
        client: &ClientImage,
        receiver: &mut ClientReceiver,
        context: &egui::Context,
        group: Messages,
        update: bool,
    ) -> (Option<[usize; 2]>, [usize; 2], Vec<u8>) {
        let total = group.len();
        for (n, (message, _)) in group.into_iter().enumerate() {
            let last = n + 1 == total;
            let (mut parser, (header, payload)) =
                MessagesParser::from_bytes(message.into_bytes()).unwrap();
            assert_eq!(header.update_flag(), last && update);
            match header {
                ServerHeader::Image(71, ImageHeader::Update(message, kind), _) => {
                    client.update_image(message, kind, &payload).unwrap();
                }
                _ => panic!("unexpected packed image update"),
            }
            assert!(parser.next().unwrap().is_none());
            if !last {
                assert!(receiver.try_recv().is_err());
                assert!(
                    crate::test_support::texture_updates(context).is_empty(),
                    "partial update was published"
                );
            }
        }
        crate::test_support::ack(receiver, 71);
        let delta = crate::test_support::texture_updates(context);
        let changes = &delta[&client.get_id().unwrap()];
        assert_eq!(changes.len(), 1);
        let egui::ImageData::Color(pixels) = &changes[0].image;
        let bytes = pixels.pixels.iter().flat_map(|p| p.to_array()).collect();
        (changes[0].pos, pixels.size, bytes)
    }

    #[test]
    fn real_image_set_and_update_chunks_preserve_pixels_and_publication_boundaries() {
        let width = 1024;
        let chunk_rows = TRANSFER_CHUNK_SIZE / (width * 4);
        for height in [
            chunk_rows - 1,
            chunk_rows,
            chunk_rows + 1,
            2 * chunk_rows + 3,
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

            // An update is chunked like a set and published once, at its last
            // chunk, with one acknowledgement.
            expected.iter_mut().for_each(|v| *v = 255 - *v);
            let image = ImageData::packed(&expected, shape, ImageType::ColorAlpha);
            let mut groups = pack_update_data(71, &[0, 0], &image, true, None).unwrap();
            assert_eq!(groups.len(), 1);
            let group = groups.pop_front().unwrap();
            assert_eq!(group.len(), expected.len().div_ceil(TRANSFER_CHUNK_SIZE));
            let (pos, size, bytes) =
                apply_update_group(&client, &mut receiver, &context, group, true);
            assert_eq!(pos, None, "an update of the whole texture replaces it");
            assert_eq!(size, [width, height]);
            assert_eq!(bytes, expected);

            // With max_lines, every part of at most that many rows is chunked
            // and published on its own, and requests a repaint.
            for max_lines in [50, 200] {
                expected.iter_mut().for_each(|v| *v = 255 - *v);
                let image = ImageData::packed(&expected, shape, ImageType::ColorAlpha);
                let groups = pack_update_data(71, &[0, 0], &image, true, Some(max_lines)).unwrap();
                assert_eq!(groups.len(), height.div_ceil(max_lines));
                let mut reconstructed = vec![0; expected.len()];
                let mut next_row = 0;
                for group in groups {
                    let (pos, size, bytes) =
                        apply_update_group(&client, &mut receiver, &context, group, true);
                    let pos = pos.unwrap_or([0, 0]);
                    assert_eq!(pos, [0, next_row]);
                    assert_eq!(size[0], width);
                    assert!(size[1] <= max_lines);
                    let offset = next_row * width * 4;
                    reconstructed[offset..offset + bytes.len()].copy_from_slice(&bytes);
                    next_row += size[1];
                }
                assert_eq!(next_row, height);
                assert_eq!(reconstructed, expected);
            }
            assert_eq!(client.get_size(), Some([width, height]));
        }
    }
}
