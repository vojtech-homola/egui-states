//! Frozen copies of the image set packers as they were before the transfer
//! refactoring, and tests that compare the live packers with them.
//!
//! The live packers may be restructured freely; for every chunk limit they must
//! produce exactly the messages and `send_now` flags of these copies. Do not
//! edit the copies unless the wire format changes on purpose. Image updates
//! are pinned to the set layout by `image_update_parts_are_chunked_like_sets`.
//! The Data and take packers changed their format for headers; their encoding
//! is pinned by `data_transfer_bytes_are_stable`.

use crate::protocol::image::{
    ImageHeader, ImageSetHeader, ImageType, checked_image_bytes, checked_image_shape,
};
use crate::protocol::{FastVec, ServerHeader};
use crate::server_core::states::image::common::checked_image_size;

/// The image descriptor the frozen packers read, independent of the live one.
pub(super) struct ReferenceImage {
    pub size: [usize; 2],
    pub stride: usize,
    pub contiguous: bool,
    pub image_type: ImageType,
    pub data: *const u8,
}

// Frozen copy of `server_core/states/image/single.rs::pack_set_data_chunked`.
pub(super) fn reference_image_set(
    id: u64,
    image: &ReferenceImage,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let [height, width] = checked_image_shape(image.size)?;
    let size = [width, height]; // reverse for egui
    checked_image_bytes(image.size, 4)?;
    let bytes_line_size = image.size[1] * image.image_type.bytes_per_pixel();
    let bytes_size = image.size[0] * bytes_line_size;

    let append_data = |message: &mut FastVec<32>, start: usize, size: usize| {
        if image.contiguous {
            let data = unsafe { std::slice::from_raw_parts(image.data.add(start), size) };
            message.extend_from_slice(data);
        } else {
            let mut processed = 0;
            while processed < size {
                let offset = start + processed;
                let line = offset / bytes_line_size;
                let line_offset = offset % bytes_line_size;
                let copy_size = (bytes_line_size - line_offset).min(size - processed);
                let data = unsafe {
                    std::slice::from_raw_parts(
                        image.data.add(line * image.stride + line_offset),
                        copy_size,
                    )
                };
                message.extend_from_slice(data);
                processed += copy_size;
            }
        }
    };

    if bytes_size <= chunk_limit {
        let header = ImageSetHeader::All(size, update);
        let mut message = header
            .serialize(id, image.image_type, bytes_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;

        message.reserve_exact(bytes_size);
        append_data(&mut message, 0, bytes_size);
        Ok(vec![(message, true)])
    } else {
        let mut messages = Vec::new();
        let pixel_size = image.image_type.bytes_per_pixel();
        let pixel_count = image.size[0] * image.size[1];
        let chunk_pixels = chunk_limit / pixel_size;
        let chunk_size = chunk_pixels * pixel_size;
        let mut processed_pixels = 0;
        let mut processed = 0;

        let first_pixels = chunk_pixels.min(pixel_count);
        let first_size = first_pixels * pixel_size;
        let header = ImageSetHeader::Start(size, first_pixels as u32);
        let mut message = header
            .serialize(id, image.image_type, first_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;
        message.reserve_exact(first_size);
        append_data(&mut message, 0, first_size);
        messages.push((message, true));
        processed_pixels += first_pixels;
        processed += first_size;

        while processed_pixels < pixel_count {
            let remaining_pixels = pixel_count - processed_pixels;
            if remaining_pixels <= chunk_pixels {
                let remaining_size = remaining_pixels * pixel_size;
                let header = ImageSetHeader::End(remaining_pixels as u32, update);
                let mut message = header
                    .serialize(id, image.image_type, remaining_size as u32)
                    .map_err(|_| format!("Failed to serialize header for image {}", id))?;
                append_data(&mut message, processed, remaining_size);
                messages.push((message, false));
                break;
            }

            let header = ImageSetHeader::Batch(chunk_pixels as u32);
            let mut message = header
                .serialize(id, image.image_type, chunk_size as u32)
                .map_err(|_| format!("Failed to serialize header for image {}", id))?;
            message.reserve_exact(chunk_size);
            append_data(&mut message, processed, chunk_size);
            messages.push((message, true));
            processed_pixels += chunk_pixels;
            processed += chunk_size;
        }

        Ok(messages)
    }
}

// Frozen copy of `server_core/states/image/multi.rs::pack_set_data_chunked`.
pub(super) fn reference_image_multi_set(
    id: u64,
    index: u32,
    image: &ReferenceImage,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let size = checked_image_size(image.size)?;
    checked_image_bytes(image.size, 4)?;
    let bytes_line_size = image.size[1] * image.image_type.bytes_per_pixel();
    let bytes_size = image.size[0] * bytes_line_size;

    let append_data = |message: &mut FastVec<32>, start: usize, size: usize| {
        if image.contiguous {
            let data = unsafe { std::slice::from_raw_parts(image.data.add(start), size) };
            message.extend_from_slice(data);
        } else {
            let mut processed = 0;
            while processed < size {
                let offset = start + processed;
                let line = offset / bytes_line_size;
                let line_offset = offset % bytes_line_size;
                let copy_size = (bytes_line_size - line_offset).min(size - processed);
                let data = unsafe {
                    std::slice::from_raw_parts(
                        image.data.add(line * image.stride + line_offset),
                        copy_size,
                    )
                };
                message.extend_from_slice(data);
                processed += copy_size;
            }
        }
    };

    if bytes_size <= chunk_limit {
        let header = ImageSetHeader::All(size, update);
        let mut message = header
            .serialize_multi(id, index, image.image_type, bytes_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;

        message.reserve_exact(bytes_size);
        append_data(&mut message, 0, bytes_size);
        Ok(vec![(message, true)])
    } else {
        let mut messages = Vec::new();
        let pixel_size = image.image_type.bytes_per_pixel();
        let pixel_count = image.size[0] * image.size[1];
        let chunk_pixels = chunk_limit / pixel_size;
        let chunk_size = chunk_pixels * pixel_size;
        let mut processed_pixels = 0;
        let mut processed = 0;

        let first_pixels = chunk_pixels.min(pixel_count);
        let first_size = first_pixels * pixel_size;
        let header = ImageSetHeader::Start(size, first_pixels as u32);
        let mut message = header
            .serialize_multi(id, index, image.image_type, first_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;
        message.reserve_exact(first_size);
        append_data(&mut message, 0, first_size);
        messages.push((message, true));
        processed_pixels += first_pixels;
        processed += first_size;

        while processed_pixels < pixel_count {
            let remaining_pixels = pixel_count - processed_pixels;
            if remaining_pixels <= chunk_pixels {
                let remaining_size = remaining_pixels * pixel_size;
                let header = ImageSetHeader::End(remaining_pixels as u32, update);
                let mut message = header
                    .serialize_multi(id, index, image.image_type, remaining_size as u32)
                    .map_err(|_| format!("Failed to serialize header for image {}", id))?;
                append_data(&mut message, processed, remaining_size);
                messages.push((message, false));
                break;
            }

            let header = ImageSetHeader::Batch(chunk_pixels as u32);
            let mut message = header
                .serialize_multi(id, index, image.image_type, chunk_size as u32)
                .map_err(|_| format!("Failed to serialize header for image {}", id))?;
            message.reserve_exact(chunk_size);
            append_data(&mut message, processed, chunk_size);
            messages.push((message, true));
            processed_pixels += chunk_pixels;
            processed += chunk_size;
        }

        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::data::{DataType, HeaderBytes, HeaderUpdate, TransportType};
    use crate::protocol::image::{ImageMultiHeader, ImageUpdateHeader};
    use crate::server_core::states::data::{WireHeader, pack_data_chunked};
    use crate::server_core::states::data_take::pack_data_take_chunked;
    use crate::server_core::states::image::multi as image_multi_core;
    use crate::server_core::states::image::single::{self as image_core, ImageData};

    type Messages = Vec<(Vec<u8>, bool)>;

    fn contents(message: &FastVec<32>) -> Vec<u8> {
        match message {
            FastVec::Heap(data) => data.clone(),
            FastVec::Stack(data) => data.as_ref().to_vec(),
        }
    }

    fn plain<'a>(messages: impl IntoIterator<Item = &'a (FastVec<32>, bool)>) -> Messages {
        messages
            .into_iter()
            .map(|(message, send_now)| (contents(message), *send_now))
            .collect()
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 251) as u8).collect()
    }

    const IMAGE_TYPES: [ImageType; 4] = [
        ImageType::Gray,
        ImageType::GrayAlpha,
        ImageType::Color,
        ImageType::ColorAlpha,
    ];
    const IMAGE_SIZES: [[usize; 2]; 6] = [[1, 1], [1, 7], [2, 3], [3, 5], [5, 2], [4, 4]];

    /// Calls `check` with the live and the reference descriptor of the same
    /// contiguous or strided pixel buffer.
    fn for_each_image(mut check: impl FnMut(&str, &ImageData, &ImageData, &ReferenceImage)) {
        for image_type in IMAGE_TYPES {
            for size in IMAGE_SIZES {
                for strided in [false, true] {
                    let line = size[1] * image_type.bytes_per_pixel();
                    let stride = if strided { line + 3 } else { line };
                    let pixels = pattern(size[0] * stride);
                    let contiguous = !strided;
                    let live = ImageData {
                        size,
                        stride,
                        image_type,
                        data: &pixels,
                    };
                    let live_multi = live;
                    let reference = ReferenceImage {
                        size,
                        stride,
                        contiguous,
                        image_type,
                        data: pixels.as_ptr(),
                    };
                    let context = format!(
                        "{} bpp size {size:?} strided {strided}",
                        image_type.bytes_per_pixel()
                    );
                    check(&context, &live, &live_multi, &reference);
                }
            }
        }
    }

    #[test]
    fn image_set_packers_match_the_frozen_reference() {
        for chunk_limit in [8, 12, 24, 64] {
            for update in [false, true] {
                for_each_image(|context, live, live_multi, reference| {
                    let context = format!("{context} chunk {chunk_limit} update {update}");
                    assert_eq!(
                        plain(
                            &image_core::pack_set_data_chunked(7, live, update, chunk_limit)
                                .unwrap()
                        ),
                        plain(&reference_image_set(7, reference, update, chunk_limit).unwrap()),
                        "{context}"
                    );
                    assert_eq!(
                        plain(
                            &image_multi_core::pack_set_data_chunked(
                                7,
                                2,
                                live_multi,
                                update,
                                chunk_limit
                            )
                            .unwrap()
                        ),
                        plain(
                            &reference_image_multi_set(7, 2, reference, update, chunk_limit)
                                .unwrap()
                        ),
                        "multi {context}"
                    );
                });
            }
        }
    }

    /// The image header and payload of a packed image message.
    fn image_message(message: &FastVec<32>) -> (ImageHeader, Vec<u8>) {
        let bytes = contents(message);
        let (header, payload) = postcard::take_from_bytes::<ServerHeader>(&bytes).unwrap();
        let (header, len) = match header {
            ServerHeader::Image(7, header, len) => (header, len),
            ServerHeader::ImageMulti(7, ImageMultiHeader::Modify(2, header, len)) => (header, len),
            _ => panic!("unexpected image message header"),
        };
        assert_eq!(len as usize, payload.len());
        (header, payload.to_vec())
    }

    /// Asserts that a message of the update part `part` (`[x, y, width,
    /// height]`) is the message of a set of the same rows, with an update
    /// header in place of the set header.
    fn assert_update_chunk(
        update: &(FastVec<32>, bool),
        set: &(FastVec<32>, bool),
        part: [u32; 4],
        context: &str,
    ) {
        assert_eq!(update.1, set.1, "{context} send_now");
        let (update_header, update_payload) = image_message(&update.0);
        let (set_header, set_payload) = image_message(&set.0);
        assert_eq!(update_payload, set_payload, "{context} payload");
        let size = [part[2], part[3]];
        let same = match (update_header, set_header) {
            (
                ImageHeader::Update(ImageUpdateHeader::All(rect, a), update_type),
                ImageHeader::Set(ImageSetHeader::All(set_size, b), set_type),
            ) => {
                rect == part
                    && set_size == size
                    && a == b
                    && update_type.bytes_per_pixel() == set_type.bytes_per_pixel()
            }
            (
                ImageHeader::Update(ImageUpdateHeader::Start(rect, a), _),
                ImageHeader::Set(ImageSetHeader::Start(set_size, b), _),
            ) => rect == part && set_size == size && a == b,
            (
                ImageHeader::Update(ImageUpdateHeader::Batch(a), _),
                ImageHeader::Set(ImageSetHeader::Batch(b), _),
            ) => a == b,
            (
                ImageHeader::Update(ImageUpdateHeader::End(a, x), _),
                ImageHeader::Set(ImageSetHeader::End(b, y), _),
            ) => a == b && x == y,
            _ => false,
        };
        assert!(same, "{context} header");
    }

    /// An image update is cut into parts of at most `max_lines` rows, and
    /// each part is chunked exactly like a set of the same rows, with its
    /// repaint flag.
    #[test]
    fn image_update_parts_are_chunked_like_sets() {
        for chunk_limit in [8, 24, 64] {
            for origin in [[0usize, 0], [1, 2]] {
                for update in [false, true] {
                    for max_lines in [None, Some(1), Some(2), Some(100)] {
                        for_each_image(|context, live, _, _| {
                            let context = format!(
                                "{context} chunk {chunk_limit} origin {origin:?} \
                                 update {update} max_lines {max_lines:?}"
                            );
                            let single = image_core::pack_update_data_chunked(
                                7,
                                &origin,
                                live,
                                update,
                                max_lines,
                                chunk_limit,
                            )
                            .unwrap();
                            let multi = image_multi_core::pack_update_data_chunked(
                                7,
                                2,
                                &origin,
                                live,
                                update,
                                max_lines,
                                chunk_limit,
                            )
                            .unwrap();
                            let rows = live.size[0];
                            let step = max_lines.unwrap_or(rows).min(rows);
                            assert_eq!(single.len(), rows.div_ceil(step), "{context}");
                            assert_eq!(multi.len(), single.len(), "{context}");

                            for (part, (group, multi_group)) in
                                single.iter().zip(&multi).enumerate()
                            {
                                let first = part * step;
                                let count = step.min(rows - first);
                                let rows_image = ImageData {
                                    size: [count, live.size[1]],
                                    stride: live.stride,
                                    image_type: live.image_type,
                                    data: &live.data[first * live.stride..],
                                };
                                let wire = [
                                    origin[1] as u32,
                                    (origin[0] + first) as u32,
                                    live.size[1] as u32,
                                    count as u32,
                                ];
                                let set = image_core::pack_set_data_chunked(
                                    7,
                                    &rows_image,
                                    update,
                                    chunk_limit,
                                )
                                .unwrap();
                                let multi_set = image_multi_core::pack_set_data_chunked(
                                    7,
                                    2,
                                    &rows_image,
                                    update,
                                    chunk_limit,
                                )
                                .unwrap();
                                assert_eq!(group.len(), set.len(), "{context} part {part}");
                                assert_eq!(multi_group.len(), set.len(), "{context} part {part}");
                                for (message, set_message) in group.iter().zip(&set) {
                                    assert_update_chunk(message, set_message, wire, &context);
                                }
                                for (message, set_message) in multi_group.iter().zip(&multi_set) {
                                    assert_update_chunk(message, set_message, wire, &context);
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    #[test]
    fn image_update_rejects_zero_max_lines() {
        let image = ImageData::packed(&[0; 4], [2, 2], ImageType::Gray);
        assert!(
            image_core::pack_update_data_chunked(7, &[0, 0], &image, false, Some(0), 64).is_err()
        );
        assert!(
            image_multi_core::pack_update_data_chunked(7, 2, &[0, 0], &image, false, Some(0), 64)
                .is_err()
        );
    }

    /// Pins the encoding of Data and take transfers: the header goes with
    /// the message that completes a transfer, in front of its elements.
    #[test]
    fn data_transfer_bytes_are_stable() {
        let data: Vec<u8> = (1..=10).collect();
        let messages = pack_data_chunked(
            5,
            &data,
            TransportType::Set(10),
            10,
            DataType::U8,
            None,
            true,
            WireHeader::KEEP,
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![
                (vec![4, 5, 1, 10, 8, 1, 2, 3, 4, 5, 6, 7, 8], true),
                (vec![4, 5, 3, 0, 0, 10, 1, 2, 0, 9, 10], false),
            ]
        );

        let messages = pack_data_chunked(
            5,
            &[1, 2],
            TransportType::Add(2),
            2,
            DataType::U8,
            Some(3),
            false,
            WireHeader::KEEP,
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![(vec![6, 5, 1, 3, 0, 0, 1, 2, 0, 2, 0, 1, 2], true)]
        );

        let header = HeaderBytes {
            type_id: 300,
            size: 3,
        };
        let messages = pack_data_chunked(
            5,
            &[1, 2],
            TransportType::Set(2),
            2,
            DataType::U8,
            None,
            false,
            WireHeader {
                update: HeaderUpdate::Set(header),
                bytes: &[7, 8, 9],
            },
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![(
                vec![4, 5, 0, 0, 0, 2, 0, 2, 1, 0xac, 0x02, 3, 7, 8, 9, 1, 2],
                true
            )]
        );

        let messages = pack_data_take_chunked(
            5,
            &[1, 2],
            2,
            DataType::U8,
            false,
            true,
            Some((header, &[7, 8, 9])),
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![(
                vec![5, 5, 0, 0, 2, 1, 2, 1, 0xac, 0x02, 3, 0, 7, 8, 9, 1, 2],
                true
            )]
        );
    }
}
