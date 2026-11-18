use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;

use crate::hashing::NoHashMap;
use crate::protocol::image::{
    ImageHeader, ImageMultiHeader, checked_image_bytes, checked_image_shape,
    serialize_image_multi_header,
};
use crate::protocol::{FastVec, TRANSFER_CHUNK_SIZE};
use crate::server_core::error::CoreError;
use crate::server_core::net::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::states::image::common::{
    ImageData, ImageFill, StoredImage, checked_image_rect, checked_image_size, checked_max_lines,
    checked_stored_size, pack_image_set, pack_image_update,
};
use crate::server_core::states::scheduler::Scheduler;
use crate::server_core::transfer::Messages;

struct ImageMultiInner {
    images: NoHashMap<u32, StoredImage>,
}

pub(crate) struct ImageMulti {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    inner: RwLock<ImageMultiInner>,
    transfer: Scheduler,
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
            }),
            transfer: Scheduler::new(id, sender, connected),
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
        self.inner.read().images.get(&index).map(StoredImage::size)
    }

    /// Calls `getter` with the image at `index`, `None` for a missing key.
    pub(crate) fn get_image<R>(
        &self,
        index: u32,
        getter: impl FnOnce(Option<&StoredImage>) -> R,
    ) -> R {
        getter(self.inner.read().images.get(&index))
    }

    /// Whether nothing is in flight and nothing waits, for any key.
    pub(crate) fn is_idle(&self) -> bool {
        self.transfer.is_idle()
    }

    /// Creates or replaces the image at `index`. Sends it once the state is
    /// idle, dropping waiting parts of a split update of the same key; with
    /// `wait`, returns once the client acknowledged it.
    pub(crate) fn set_image(
        &self,
        index: u32,
        image: ImageData,
        update: bool,
        wait: bool,
    ) -> Result<(), String> {
        checked_stored_size(image.size)?;
        let to_send = if self.transfer.is_connected() {
            Some(pack_set_data(self.transfer.id, index, &image, update)?)
        } else {
            None
        };
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        let stored = inner.images.entry(index);
        stored.or_insert_with(StoredImage::empty).set(&image);
        let Some(generation) = self.transfer.sendable() else {
            return Ok(());
        };
        let to_send = match to_send {
            Some(messages) => messages,
            None => pack_set_data(self.transfer.id, index, &image, update)?,
        };
        drop(inner);
        self.transfer
            .send_set(lock, index, to_send, wait, generation);
        Ok(())
    }

    /// Fills the image at `index` with `fill`, stored in its format; the
    /// client gets only the color.
    pub(crate) fn set_all_image(
        &self,
        index: u32,
        size: [usize; 2],
        fill: ImageFill,
        update: bool,
        wait: bool,
    ) -> Result<(), String> {
        let rgba = fill.rgba();
        let wire_size = checked_stored_size(size)?;
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
        let stored = inner.images.entry(index);
        stored.or_insert_with(StoredImage::empty).fill(size, fill);
        let Some(generation) = self.transfer.sendable() else {
            return Ok(());
        };
        let to_send = match to_send {
            Some(messages) => messages,
            None => pack_fill_data(self.transfer.id, index, wire_size, rgba, update)?,
        };
        drop(inner);
        self.transfer
            .send_set(lock, index, to_send, wait, generation);
        Ok(())
    }

    /// Writes a rectangle of the image at `index` and sends it as
    /// `Image::update_image` does; `force` only drops waiting parts of the
    /// same key.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_image(
        &self,
        index: u32,
        origin: &[usize; 2],
        image: ImageData,
        update: bool,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<(), CoreError> {
        checked_stored_size(image.size)?;
        checked_image_rect(origin, image.size)?;
        checked_max_lines(max_lines)?;
        let to_send = if self.transfer.is_connected() {
            Some(pack_update_data(
                self.transfer.id,
                index,
                origin,
                &image,
                update,
                max_lines,
            )?)
        } else {
            None
        };
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        let stored = inner
            .images
            .get_mut(&index)
            .ok_or_else(|| CoreError::not_found("ImageMulti index not found."))?;
        if !stored.contains(origin, image.size) {
            return Err(format!("ImageMulti index {} update exceeds bounds", index).into());
        }
        // Widens the stored image first when the update has more channels.
        stored.update(&image, origin);
        let Some(generation) = self.transfer.sendable() else {
            return Ok(());
        };
        let to_send = match to_send {
            Some(messages) => messages,
            None => pack_update_data(self.transfer.id, index, origin, &image, update, max_lines)?,
        };
        let rect = [origin[0], origin[1], image.size[0], image.size[1]];
        drop(inner);
        self.transfer
            .send_update(lock, index, rect, to_send, force, wait, generation);
        Ok(())
    }

    pub(crate) fn remove_index(&self, index: u32, update: bool) -> Result<(), String> {
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        if inner.images.remove(&index).is_none() {
            return Ok(());
        }
        let Some(generation) = self.transfer.sendable() else {
            return Ok(());
        };
        drop(inner);
        let message = ImageMultiHeader::Remove(index, update)
            .serialize(self.transfer.id)
            .map_err(|_| "Failed to serialize ImageMulti remove header".to_string())?;
        self.transfer
            .send_control(lock, Some(index), message, generation);
        Ok(())
    }

    pub(crate) fn reset_images(&self, update: bool) -> Result<(), String> {
        let lock = self.transfer.lock.lock();
        let mut inner = self.inner.write();
        if inner.images.is_empty() {
            return Ok(());
        }
        inner.images.clear();
        let Some(generation) = self.transfer.sendable() else {
            return Ok(());
        };
        drop(inner);
        let message = ImageMultiHeader::Reset(update)
            .serialize(self.transfer.id)
            .map_err(|_| "Failed to serialize ImageMulti reset header".to_string())?;
        self.transfer.send_control(lock, None, message, generation);
        Ok(())
    }
}

impl Acknowledge for ImageMulti {
    fn acknowledge(&self) {
        self.transfer.acknowledge();
    }

    fn reset(&self) {
        // Under the storage lock, so a producer either checked whether it may
        // send before this reset, or sees the new state.
        let _inner = self.inner.write();
        self.transfer.reset();
    }
}

impl SyncTrait for ImageMulti {
    fn sync(&self) -> Result<(), ()> {
        let _lock = self.transfer.lock.lock();
        let inner = self.inner.write();
        if !self.transfer.required() {
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
            // Sent in the stored format; the client expands it to RGBA.
            messages.extend(
                pack_set_data(self.transfer.id, index, &image.image_data(), false)
                    .map_err(|_| ())?,
            );
        }
        let ack_count = inner.images.len();
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
    max_lines: Option<usize>,
) -> Result<VecDeque<Messages>, String> {
    pack_update_data_chunked(
        id,
        index,
        origin,
        image,
        update,
        max_lines,
        TRANSFER_CHUNK_SIZE,
    )
}

pub(in crate::server_core) fn pack_update_data_chunked(
    id: u64,
    index: u32,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
    max_lines: Option<usize>,
    chunk_limit: usize,
) -> Result<VecDeque<Messages>, String> {
    let wire_rect = checked_image_rect(origin, image.size)?;
    checked_image_bytes(image.size, 4)?;
    pack_image_update(
        id,
        image,
        wire_rect,
        update,
        max_lines,
        chunk_limit,
        |header, len| serialize_image_multi_header(id, index, header, len),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc::error::TryRecvError;

    use super::*;
    #[cfg(feature = "client")]
    use crate::protocol::ServerHeader;
    use crate::protocol::image::ImageType;
    #[cfg(feature = "client")]
    use crate::protocol::image::{ImageHeader, ImageSetHeader, ImageUpdateHeader};
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
            images
                .set_all_image(7, [1, 1], ImageFill::ColorAlpha([9; 4]), false, false)
                .unwrap();
            for size in [[65_537, 1], [1, 65_537]] {
                let data = || image_data(&[], size, ImageType::Gray);
                for result in [
                    images.set_image(7, data(), false, false),
                    images.set_image(9, data(), false, false),
                    images.set_all_image(7, size, ImageFill::ColorAlpha([0; 4]), false, false),
                    images
                        .update_image(7, &[0, 0], data(), false, false, None, false)
                        .map_err(String::from),
                    pack_set_data(1, 7, &data(), false).map(|_| ()),
                    pack_fill_data(1, 7, [size[0] as u32, size[1] as u32], [0; 4], false)
                        .map(|_| ()),
                    pack_update_data(1, 7, &[0, 0], &data(), false, None).map(|_| ()),
                ] {
                    assert!(result.unwrap_err().contains("65536"));
                }
            }
            for origin in [[65_536, 0], [0, 65_536], [usize::MAX, 0]] {
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(
                    images
                        .update_image(7, &origin, data, false, false, None, false)
                        .is_err()
                );
                let data = image_data(&[0], [1, 1], ImageType::Gray);
                assert!(pack_update_data(1, 7, &origin, &data, false, None).is_err());
            }
            assert_eq!(
                images.get_image(7, |image| image
                    .map(|image| (image.data().to_vec(), image.size()))),
                Some((vec![9; 4], [1, 1]))
            );
            assert_eq!(images.indices(), vec![7]);
            assert!(images.is_idle());
            assert_no_message(&mut receiver);
        }
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
            .set_image(
                4,
                image_data(&first, [1, 2], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        // Pointer stability is a limited allocation regression, not an allocation counter.
        let allocation = images.inner.read().images[&4].data().as_ptr();

        let second = [8, 7, 6, 5, 4, 3, 2, 1];
        images
            .set_image(
                4,
                image_data(&second, [1, 2], ImageType::ColorAlpha),
                false,
                false,
            )
            .unwrap();
        assert_eq!(images.inner.read().images[&4].data().as_ptr(), allocation);
        assert_eq!(images.get_image(4, |v| v.unwrap().data().to_vec()), second);

        images
            .set_all_image(4, [1, 2], ImageFill::ColorAlpha([9, 8, 7, 6]), false, false)
            .unwrap();
        assert_eq!(images.inner.read().images[&4].data().as_ptr(), allocation);
        assert_eq!(
            images.get_image(4, |v| v.unwrap().data().to_vec()),
            [9, 8, 7, 6].repeat(2)
        );
    }

    #[test]
    fn invalid_dimensions_do_not_mutate_or_send() {
        let (images, _, mut receiver) = new_image_multi(true);
        let empty = [];
        assert!(
            images
                .set_image(1, image_data(&empty, [0, 1], ImageType::Gray), false, false)
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
                        false,
                    )
                    .unwrap_err()
                    .contains("65536")
            );
            assert!(!images.contains(1));
            assert_no_message(&mut receiver);
        }

        images
            .set_all_image(1, [1, 1], ImageFill::ColorAlpha([1, 2, 3, 4]), false, false)
            .unwrap();
        let before = images.get_image(1, |image| image.unwrap().data().to_vec());
        assert!(
            images
                .update_image(
                    1,
                    &[0, 0],
                    image_data(&empty, [1, 0], ImageType::Gray),
                    false,
                    false,
                    None,
                    false,
                )
                .is_err()
        );
        assert_eq!(
            images.get_image(1, |image| image.unwrap().data().to_vec()),
            before
        );
        assert_no_message(&mut receiver);
    }

    #[test]
    #[cfg(feature = "client")]
    fn sync_resets_sorts_and_waits_for_every_key_ack() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        images
            .set_all_image(
                7,
                [1, 1],
                ImageFill::ColorAlpha([7, 7, 7, 255]),
                false,
                false,
            )
            .unwrap();
        images
            .set_all_image(2, [1, 1], ImageFill::Gray([2]), false, false)
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

        // Every key is sent in its own stored format.
        for (expected_index, expected_type, expected_pixels) in [
            (2, ImageType::Gray, &[2][..]),
            (7, ImageType::ColorAlpha, &[7, 7, 7, 255]),
        ] {
            let (message, _) = receiver.try_recv().unwrap().unwrap();
            let bytes = message.into_bytes();
            let (header, header_size) = ServerHeader::deserialize(&bytes).unwrap();
            match header {
                ServerHeader::ImageMulti(
                    1,
                    ImageMultiHeader::Modify(
                        index,
                        ImageHeader::Set(ImageSetHeader::All([1, 1], false), image_type),
                        size,
                    ),
                ) => {
                    assert_eq!(index, expected_index);
                    assert_eq!(image_type, expected_type);
                    assert_eq!(size as usize, expected_pixels.len());
                }
                _ => panic!("unexpected ImageMulti sync header"),
            }
            assert_eq!(&bytes[header_size..], expected_pixels);
        }
        assert_no_message(&mut receiver);
        assert!(!images.is_idle());

        // A set waits until every image of the synchronization is acknowledged.
        let waiting_images = images.clone();
        let waiter = std::thread::spawn(move || {
            waiting_images.set_all_image(
                9,
                [1, 1],
                ImageFill::ColorAlpha([9, 9, 9, 255]),
                true,
                false,
            )
        });
        wait_until(|| images.contains(9));
        assert_no_message(&mut receiver);
        images.acknowledge();
        assert_no_message(&mut receiver);
        images.acknowledge();
        waiter.join().unwrap().unwrap();

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
        assert!(images.is_idle());
    }

    #[test]
    #[cfg(feature = "client")]
    fn reconnect_sync_reproduces_only_current_keys() {
        let (images, connected, mut receiver) = new_image_multi(false);
        for index in [8, 3, 5] {
            images
                .set_all_image(
                    index,
                    [1, 1],
                    ImageFill::ColorAlpha([index as u8; 4]),
                    false,
                    false,
                )
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
        images
            .set_all_image(13, [1, 1], ImageFill::ColorAlpha([13; 4]), false, false)
            .unwrap();
        images
            .set_all_image(1, [1, 1], ImageFill::ColorAlpha([1; 4]), false, false)
            .unwrap();

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

    #[test]
    fn remove_and_reset_controls_leave_transfer_ready_for_later_sets() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        images
            .set_all_image(
                1,
                [1, 1],
                ImageFill::ColorAlpha([1, 1, 1, 255]),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        assert_message(&mut receiver); // initial image
        images.acknowledge();

        images
            .set_all_image(
                1,
                [1, 1],
                ImageFill::ColorAlpha([2, 2, 2, 255]),
                false,
                false,
            )
            .unwrap();
        assert_message(&mut receiver);
        let removing_images = images.clone();
        let remove = std::thread::spawn(move || removing_images.remove_index(1, false));
        wait_until(|| !images.contains(1));
        assert_no_message(&mut receiver);
        images.acknowledge();
        remove.join().unwrap().unwrap();
        assert_message(&mut receiver); // remove
        assert!(images.is_idle());

        images
            .set_all_image(
                2,
                [1, 1],
                ImageFill::ColorAlpha([3, 3, 3, 255]),
                false,
                false,
            )
            .unwrap();
        assert_message(&mut receiver);
        let resetting_images = images.clone();
        let reset = std::thread::spawn(move || resetting_images.reset_images(false));
        wait_until(|| images.len() == 0);
        assert_no_message(&mut receiver);
        images.acknowledge();
        reset.join().unwrap().unwrap();
        assert_message(&mut receiver); // reset
        assert!(images.is_idle());

        images
            .set_all_image(
                3,
                [1, 1],
                ImageFill::ColorAlpha([4, 4, 4, 255]),
                false,
                false,
            )
            .unwrap();
        assert_message(&mut receiver);
        images.acknowledge();
        assert!(images.is_idle());
    }

    #[test]
    fn reset_discards_a_set_waiter_from_the_previous_connection() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        images
            .set_all_image(
                1,
                [1, 1],
                ImageFill::ColorAlpha([1, 1, 1, 255]),
                false,
                false,
            )
            .unwrap();
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        assert_message(&mut receiver); // reset
        assert_message(&mut receiver); // initial image

        // The synchronization is not acknowledged, so this set waits.
        let waiting_images = images.clone();
        let waiter = std::thread::spawn(move || {
            waiting_images.set_all_image(
                1,
                [1, 1],
                ImageFill::ColorAlpha([3, 3, 3, 255]),
                false,
                false,
            )
        });
        wait_until(|| images.get_image(1, |image| image.unwrap().data() == [3, 3, 3, 255]));
        assert_no_message(&mut receiver);

        connected.store(false, Ordering::Release);
        images.reset();
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        waiter.join().unwrap().unwrap();

        assert_message(&mut receiver); // reconnect reset
        assert_message(&mut receiver); // latest complete image
        assert_no_message(&mut receiver);
    }

    /// Checks that the next message updates `index` with `expected` gray
    /// pixels and requests no repaint.
    #[cfg(feature = "client")]
    fn assert_update(receiver: &mut MessageReceiver, index: u32, expected: &[u8]) {
        let (message, _) = receiver.try_recv().unwrap().unwrap();
        let bytes = message.into_bytes();
        let (header, header_size) = ServerHeader::deserialize(&bytes).unwrap();
        match header {
            ServerHeader::ImageMulti(
                1,
                ImageMultiHeader::Modify(
                    sent,
                    ImageHeader::Update(ImageUpdateHeader::All(_, false), ImageType::Gray),
                    _,
                ),
            ) => assert_eq!(sent, index),
            _ => panic!("unexpected ImageMulti update header"),
        }
        assert_eq!(&bytes[header_size..], expected);
    }

    /// A forced update compares its rectangle with the whole earlier update
    /// and drops only waiting parts of its own index.
    #[test]
    #[cfg(feature = "client")]
    fn a_forced_update_drops_only_covered_parts_of_its_own_index() {
        if crate::test_support::isolated() {
            return;
        }
        let (images, connected, mut receiver) = new_image_multi(false);
        for index in [1, 2] {
            images
                .set_all_image(
                    index,
                    [2, 1],
                    ImageFill::ColorAlpha([0, 0, 0, 255]),
                    false,
                    false,
                )
                .unwrap();
        }
        connected.store(true, Ordering::Release);
        images.sync().unwrap();
        for _ in 0..3 {
            assert_message(&mut receiver);
        }
        images.acknowledge();
        images.acknowledge();
        let column = |index: u32| images.get_image(index, |image| image.unwrap().data().to_vec());

        // A split update of index 1: the first row goes out, the second waits.
        let rows = [10, 11];
        images
            .update_image(
                1,
                &[0, 0],
                image_data(&rows, [2, 1], ImageType::Gray),
                false,
                false,
                Some(1),
                false,
            )
            .unwrap();
        assert_update(&mut receiver, 1, &[10]);

        // A forced update of index 2 does not drop that row.
        let waiting_images = images.clone();
        let other_index = std::thread::spawn(move || {
            let rows = [20, 21];
            waiting_images.update_image(
                2,
                &[0, 0],
                image_data(&rows, [2, 1], ImageType::Gray),
                false,
                true,
                None,
                false,
            )
        });
        wait_until(|| column(2) == [20, 20, 20, 255, 21, 21, 21, 255]);
        assert_no_message(&mut receiver);
        images.acknowledge();
        assert_update(&mut receiver, 1, &[11]);
        images.acknowledge();
        other_index.join().unwrap().unwrap();
        assert_update(&mut receiver, 2, &[20, 21]);
        images.acknowledge();

        // A forced update of index 1 that covers the waiting row drops it.
        let rows = [30, 31];
        images
            .update_image(
                1,
                &[0, 0],
                image_data(&rows, [2, 1], ImageType::Gray),
                false,
                false,
                Some(1),
                false,
            )
            .unwrap();
        assert_update(&mut receiver, 1, &[30]);
        let waiting_images = images.clone();
        let same_index = std::thread::spawn(move || {
            let rows = [40, 41];
            waiting_images.update_image(
                1,
                &[0, 0],
                image_data(&rows, [2, 1], ImageType::Gray),
                false,
                true,
                None,
                false,
            )
        });
        wait_until(|| column(1) == [40, 40, 40, 255, 41, 41, 41, 255]);
        // The stored pixels change before the waiting row is dropped; an
        // acknowledgement in between would send that row and leave the
        // forced update waiting for an acknowledgement that never comes.
        wait_until(|| !images.transfer.has_waiting());
        images.acknowledge();
        same_index.join().unwrap().unwrap();
        assert_update(&mut receiver, 1, &[40, 41]);
        assert_no_message(&mut receiver);
        images.acknowledge();
        assert!(images.is_idle());
    }
}
