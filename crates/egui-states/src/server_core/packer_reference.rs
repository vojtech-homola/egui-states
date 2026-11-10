//! Frozen copies of the chunk packers as they were before the transfer
//! refactoring, and tests that compare the live packers with them.
//!
//! The live packers may be restructured freely; for every chunk limit they must
//! produce exactly the messages and `send_now` flags of these copies. Do not
//! edit the copies unless the wire format changes on purpose.

use std::collections::VecDeque;

use crate::data_transport::{
    DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, MultiDataHeader, TransportType,
};
use crate::image_transport::{
    ImageHeader, ImageSetHeader, ImageType, checked_image_bounds, checked_image_bytes,
    checked_image_shape, serialize_image_multi_header,
};
use crate::serialization::{FastVec, ServerHeader};
use crate::server_core::image_core_common::{checked_image_rect, checked_image_size};

/// The image descriptor the frozen packers read, independent of the live one.
pub(super) struct ReferenceImage {
    pub size: [usize; 2],
    pub stride: usize,
    pub contiguous: bool,
    pub image_type: ImageType,
    pub data: *const u8,
}

// Frozen copy of `server_core/data_core.rs::pack_data_chunked`.
#[allow(clippy::too_many_arguments)]
pub(super) fn reference_data(
    id: u64,
    data: &[u8],
    transport_type: TransportType,
    data_count: u64,
    data_type: DataType,
    multi_data: Option<u32>,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let mut messages = Vec::with_capacity(1);

    if data.len() <= chunk_limit {
        let header = DataHeader::All(data_type, transport_type, update, data.len() as u32);
        let mut message = match multi_data {
            None => header.serialize(id, true),
            Some(index) => MultiDataHeader::serialize_modify(id, index, header),
        }
        .map_err(|_| "Failed to serialize header".to_string())?;
        message.reserve_exact(data.len());
        message.extend_from_slice(data);
        messages.push((message, true));
    } else {
        let element_size = data_type.item_size();
        let chunk_size = chunk_limit / element_size * element_size;
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

// Frozen copy of `server_core/data_take_core.rs::pack_data_take_chunked`.
pub(super) fn reference_data_take(
    id: u64,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let mut messages = Vec::with_capacity(1);

    if data.len() <= chunk_limit {
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
        message.extend_from_slice(data);
        messages.push((message, true));
    } else {
        let element_size = data_type.item_size();
        let chunk_size = chunk_limit / element_size * element_size;
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

// Frozen copy of `server_core/data_take_core.rs::pack_data_multi_take_chunked`.
#[allow(clippy::too_many_arguments)]
pub(super) fn reference_data_multi_take(
    id: u64,
    index: u32,
    data: &[u8],
    data_count: u64,
    data_type: DataType,
    blocking: bool,
    update: bool,
    chunk_limit: usize,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let mut messages = Vec::with_capacity(1);

    if data.len() <= chunk_limit {
        let header = DataTakeHeader::All(data_type, data_count, update, data.len() as u32);
        let mut message = DataMultiTakeHeader::serialize_modify(id, index, header, blocking)
            .map_err(|_| "Failed to serialize DataMultiTake header".to_string())?;
        message.reserve_exact(data.len());
        message.extend_from_slice(data);
        messages.push((message, true));
    } else {
        let element_size = data_type.item_size();
        let chunk_size = chunk_limit / element_size * element_size;
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

// Frozen copy of `server_core/image_core.rs::pack_set_data_chunked`.
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

// Frozen copy of `server_core/image_core.rs::pack_update_data_chunked`.
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
        let mut message: FastVec<32> = crate::serialization::serialize_heap(&header)
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
            let mut message: FastVec<32> = crate::serialization::serialize_heap(&header)
                .map_err(|_| format!("Failed to serialize update header for image {}", id))?;
            message.reserve_exact(data_size);
            append_lines(&mut message, processed_lines, lines);
            messages.push_back((message, !is_last));
            processed_lines += lines;
        }

        Ok(messages)
    }
}

// Frozen copy of `server_core/image_multi_core.rs::pack_set_data_chunked`.
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

// Frozen copy of `server_core/image_multi_core.rs::pack_update_data_chunked`.
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
    use crate::server_core::data_core::pack_data_chunked;
    use crate::server_core::data_take_core::{
        pack_data_multi_take_chunked, pack_data_take_chunked,
    };
    use crate::server_core::image_core::{self, ImageData};
    use crate::server_core::image_multi_core;

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

    const CHUNK_LIMITS: [usize; 3] = [8, 12, 24];
    const DATA_TYPES: [DataType; 4] = [DataType::U8, DataType::U16, DataType::U32, DataType::F64];

    /// Element counts around the chunk boundary: empty, one, c - 1, c, c + 1
    /// and a remainder after two full chunks.
    fn counts(chunk_limit: usize, item_size: usize) -> Vec<usize> {
        let chunk = (chunk_limit / item_size).max(1);
        let mut counts = vec![0, 1, chunk - 1, chunk, chunk + 1, 2 * chunk + chunk / 2 + 1];
        counts.sort_unstable();
        counts.dedup();
        counts
    }

    fn transport(kind: usize, count: u64) -> TransportType {
        match kind {
            0 => TransportType::Set(count),
            1 => TransportType::Add(count),
            _ => TransportType::Replace(2, count),
        }
    }

    #[test]
    fn data_packers_match_the_frozen_reference() {
        for chunk_limit in CHUNK_LIMITS {
            for data_type in DATA_TYPES {
                for count in counts(chunk_limit, data_type.item_size()) {
                    let data = pattern(count * data_type.item_size());
                    for multi in [None, Some(3)] {
                        for update in [false, true] {
                            for kind in 0..3 {
                                let count = count as u64;
                                let live = pack_data_chunked(
                                    5,
                                    &data,
                                    transport(kind, count),
                                    count,
                                    data_type,
                                    multi,
                                    update,
                                    chunk_limit,
                                )
                                .unwrap();
                                let reference = reference_data(
                                    5,
                                    &data,
                                    transport(kind, count),
                                    count,
                                    data_type,
                                    multi,
                                    update,
                                    chunk_limit,
                                )
                                .unwrap();
                                assert_eq!(
                                    plain(&live),
                                    plain(&reference),
                                    "chunk {chunk_limit} {data_type:?} count {count} \
                                     multi {multi:?} update {update} transport {kind}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn data_take_packers_match_the_frozen_reference() {
        for chunk_limit in CHUNK_LIMITS {
            for data_type in DATA_TYPES {
                for count in counts(chunk_limit, data_type.item_size()) {
                    let data = pattern(count * data_type.item_size());
                    let count = count as u64;
                    for blocking in [false, true] {
                        for update in [false, true] {
                            let context = format!(
                                "chunk {chunk_limit} {data_type:?} count {count} \
                                 blocking {blocking} update {update}"
                            );
                            let live = pack_data_take_chunked(
                                5,
                                &data,
                                count,
                                data_type,
                                blocking,
                                update,
                                chunk_limit,
                            )
                            .unwrap();
                            let reference = reference_data_take(
                                5,
                                &data,
                                count,
                                data_type,
                                blocking,
                                update,
                                chunk_limit,
                            )
                            .unwrap();
                            assert_eq!(plain(&live), plain(&reference), "{context}");

                            let live = pack_data_multi_take_chunked(
                                5,
                                3,
                                &data,
                                count,
                                data_type,
                                blocking,
                                update,
                                chunk_limit,
                            )
                            .unwrap();
                            let reference = reference_data_multi_take(
                                5,
                                3,
                                &data,
                                count,
                                data_type,
                                blocking,
                                update,
                                chunk_limit,
                            )
                            .unwrap();
                            assert_eq!(plain(&live), plain(&reference), "multi {context}");
                        }
                    }
                }
            }
        }
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
                        contiguous,
                        image_type,
                        data: pixels.as_ptr(),
                    };
                    let live_multi = ImageData {
                        size,
                        stride,
                        contiguous,
                        image_type,
                        data: pixels.as_ptr(),
                    };
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

    /// Pins the encoding of a chunked transfer independently of the reference.
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
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![
                (vec![4, 5, 1, 10, 8, 1, 2, 3, 4, 5, 6, 7, 8], true),
                (vec![4, 5, 3, 0, 0, 10, 1, 2, 9, 10], false),
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
            8,
        )
        .unwrap();
        assert_eq!(
            plain(&messages),
            vec![(vec![6, 5, 1, 3, 0, 0, 1, 2, 0, 2, 1, 2], true)]
        );
    }
}
