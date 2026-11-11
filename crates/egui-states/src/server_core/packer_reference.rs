//! Frozen copies of the image packers as they were before the transfer
//! refactoring, and tests that compare the live packers with them.
//!
//! The live packers may be restructured freely; for every chunk limit they must
//! produce exactly the messages and `send_now` flags of these copies. Do not
//! edit the copies unless the wire format changes on purpose. The Data and
//! take packers changed their format for headers; their encoding is pinned
//! by `data_transfer_bytes_are_stable`.

use std::collections::VecDeque;

use crate::protocol::image::{
    ImageHeader, ImageSetHeader, ImageType, checked_image_bounds, checked_image_bytes,
    checked_image_shape, serialize_image_multi_header,
};
use crate::protocol::{FastVec, ServerHeader};
use crate::server_core::states::image::common::{checked_image_rect, checked_image_size};

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

// Frozen copy of `server_core/states/image/single.rs::pack_update_data_chunked`.
pub(super) fn reference_image_update(
    id: u64,
    origin: &[usize; 2],
    image: &ReferenceImage,
    update: bool,
    chunk_limit: usize,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    checked_image_bounds(*origin, image.size)?;
    checked_image_bytes(image.size, 4)?;
    let [height, width] = checked_image_shape(image.size)?;
    let [y, x] = checked_image_shape(*origin)?;
    let bytes_line_size = image.size[1] * image.image_type.bytes_per_pixel();
    let bytes_size = image.size[0] * bytes_line_size;

    let append_lines = |message: &mut FastVec<32>, start_line: usize, lines: usize| {
        if image.contiguous {
            let offset = start_line * bytes_line_size;
            let size = lines * bytes_line_size;
            let data = unsafe { std::slice::from_raw_parts(image.data.add(offset), size) };
            message.extend_from_slice(data);
        } else {
            for line in start_line..start_line + lines {
                let data = unsafe {
                    std::slice::from_raw_parts(image.data.add(line * image.stride), bytes_line_size)
                };
                message.extend_from_slice(data);
            }
        }
    };

    if bytes_size <= chunk_limit {
        // swap for egui
        let rect = [x, y, width, height];
        let header = ServerHeader::Image(
            id,
            ImageHeader::Update(rect, image.image_type, update),
            bytes_size as u32,
        );
        let mut message: FastVec<32> = crate::protocol::serialize_heap(&header)
            .map_err(|_| format!("Failed to serialize update header for image {}", id))?;

        message.reserve_exact(bytes_size);
        append_lines(&mut message, 0, image.size[0]);
        let mut result = VecDeque::with_capacity(1);
        result.push_back((message, update));
        Ok(result)
    } else {
        let mut messages = VecDeque::new();
        let chunk_lines = (chunk_limit / bytes_line_size).max(1);
        let mut processed_lines = 0;

        while processed_lines < image.size[0] {
            let remaining_lines = image.size[0] - processed_lines;
            let lines = remaining_lines.min(chunk_lines);
            let data_size = lines * bytes_line_size;
            let is_last = lines == remaining_lines;
            let rect = [x, y + processed_lines as u32, width, lines as u32];
            let header = ServerHeader::Image(
                id,
                ImageHeader::Update(rect, image.image_type, is_last && update),
                data_size as u32,
            );
            let mut message: FastVec<32> = crate::protocol::serialize_heap(&header)
                .map_err(|_| format!("Failed to serialize update header for image {}", id))?;
            message.reserve_exact(data_size);
            append_lines(&mut message, processed_lines, lines);
            messages.push_back((message, !is_last));
            processed_lines += lines;
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

// Frozen copy of `server_core/states/image/multi.rs::pack_update_data_chunked`.
pub(super) fn reference_image_multi_update(
    id: u64,
    index: u32,
    origin: &[usize; 2],
    image: &ReferenceImage,
    update: bool,
    chunk_limit: usize,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    let wire_rect = checked_image_rect(origin, image.size)?;
    checked_image_bytes(image.size, 4)?;
    let bytes_line_size = image.size[1] * image.image_type.bytes_per_pixel();
    let bytes_size = image.size[0] * bytes_line_size;

    let append_lines = |message: &mut FastVec<32>, start_line: usize, lines: usize| {
        if image.contiguous {
            let offset = start_line * bytes_line_size;
            let size = lines * bytes_line_size;
            let data = unsafe { std::slice::from_raw_parts(image.data.add(offset), size) };
            message.extend_from_slice(data);
        } else {
            for line in start_line..start_line + lines {
                let data = unsafe {
                    std::slice::from_raw_parts(image.data.add(line * image.stride), bytes_line_size)
                };
                message.extend_from_slice(data);
            }
        }
    };

    if bytes_size <= chunk_limit {
        let mut message = serialize_image_multi_header(
            id,
            index,
            ImageHeader::Update(wire_rect, image.image_type, update),
            bytes_size as u32,
        )
        .map_err(|_| format!("Failed to serialize update header for image {}", id))?;

        message.reserve_exact(bytes_size);
        append_lines(&mut message, 0, image.size[0]);
        let mut result = VecDeque::with_capacity(1);
        result.push_back((message, update));
        Ok(result)
    } else {
        let mut messages = VecDeque::new();
        let chunk_lines = (chunk_limit / bytes_line_size).max(1);
        let mut processed_lines = 0;

        while processed_lines < image.size[0] {
            let remaining_lines = image.size[0] - processed_lines;
            let lines = remaining_lines.min(chunk_lines);
            let data_size = lines * bytes_line_size;
            let is_last = lines == remaining_lines;
            let wire_processed_lines = u32::try_from(processed_lines)
                .map_err(|_| "Image coordinates exceed protocol limits".to_string())?;
            let rect = [
                wire_rect[0],
                wire_rect[1] + wire_processed_lines,
                wire_rect[2],
                u32::try_from(lines)
                    .map_err(|_| "Image dimensions exceed protocol limits".to_string())?,
            ];
            let mut message = serialize_image_multi_header(
                id,
                index,
                ImageHeader::Update(rect, image.image_type, is_last && update),
                data_size as u32,
            )
            .map_err(|_| format!("Failed to serialize update header for image {}", id))?;
            message.reserve_exact(data_size);
            append_lines(&mut message, processed_lines, lines);
            messages.push_back((message, !is_last));
            processed_lines += lines;
        }

        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::data::{DataType, HeaderBytes, HeaderUpdate, TransportType};
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

    /// Image updates are chunked by rows; a single-message update is queued
    /// with `send_now = update`, chunks with `send_now = !is_last`.
    #[test]
    fn image_update_packers_match_the_frozen_reference() {
        for chunk_limit in [12, 64] {
            for origin in [[0, 0], [1, 2]] {
                for update in [false, true] {
                    for_each_image(|context, live, live_multi, reference| {
                        let context = format!(
                            "{context} chunk {chunk_limit} origin {origin:?} update {update}"
                        );
                        assert_eq!(
                            plain(
                                &image_core::pack_update_data_chunked(
                                    7,
                                    &origin,
                                    live,
                                    update,
                                    chunk_limit
                                )
                                .unwrap()
                            ),
                            plain(
                                &reference_image_update(7, &origin, reference, update, chunk_limit)
                                    .unwrap()
                            ),
                            "{context}"
                        );
                        assert_eq!(
                            plain(
                                &image_multi_core::pack_update_data_chunked(
                                    7,
                                    2,
                                    &origin,
                                    live_multi,
                                    update,
                                    chunk_limit
                                )
                                .unwrap()
                            ),
                            plain(
                                &reference_image_multi_update(
                                    7,
                                    2,
                                    &origin,
                                    reference,
                                    update,
                                    chunk_limit
                                )
                                .unwrap()
                            ),
                            "multi {context}"
                        );
                    });
                }
            }
        }
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
            WireHeader::DEFAULT,
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![(vec![6, 5, 1, 3, 0, 0, 1, 2, 0, 2, 1, 1, 2], true)]
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
                vec![4, 5, 0, 0, 0, 2, 0, 2, 2, 0xac, 0x02, 3, 7, 8, 9, 1, 2],
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
