use std::collections::VecDeque;

use std::mem::MaybeUninit;

use crate::image_transport::{
    ImageHeader, ImageSetHeader, ImageType, checked_image_bounds, checked_image_bytes,
    checked_image_shape, to_rgba,
};
use crate::serialization::FastVec;
use crate::server_core::transfer::{Chunk, Messages, chunk_rows, chunk_transfer};

/// A borrowed source image in one of the accepted pixel formats.
///
/// `size` is `[height, width]`. Rows are `stride` bytes apart; the stride is
/// read only when the rows are not `contiguous`.
pub(crate) struct ImageData {
    pub size: [usize; 2],
    pub stride: usize,
    pub contiguous: bool,
    pub image_type: ImageType,
    pub data: *const u8,
}

impl ImageData {
    fn row_bytes(&self) -> usize {
        self.size[1] * self.image_type.bytes_per_pixel()
    }

    /// Appends `len` pixel bytes starting `start` bytes into the image, as if
    /// its rows were contiguous.
    fn append(&self, message: &mut FastVec<32>, start: usize, len: usize) {
        if self.contiguous {
            let data = unsafe { std::slice::from_raw_parts(self.data.add(start), len) };
            message.extend_from_slice(data);
            return;
        }

        let row_bytes = self.row_bytes();
        let mut processed = 0;
        while processed < len {
            let offset = start + processed;
            let row = offset / row_bytes;
            let row_offset = offset % row_bytes;
            let copy = (row_bytes - row_offset).min(len - processed);
            let data = unsafe {
                std::slice::from_raw_parts(self.data.add(row * self.stride + row_offset), copy)
            };
            message.extend_from_slice(data);
            processed += copy;
        }
    }
}

/// Packs a whole-image set of the validated `image`, whose wire size is
/// `size` (`[width, height]`). `header` serializes a set header with its
/// payload length for the image's state.
pub(crate) fn pack_image_set(
    id: u64,
    image: &ImageData,
    size: [u32; 2],
    update: bool,
    chunk_limit: usize,
    header: impl Fn(ImageSetHeader, u32) -> Result<FastVec<32>, ()>,
) -> Result<Messages, String> {
    let pixel_size = image.image_type.bytes_per_pixel();
    let total = image.size[0] * image.row_bytes();
    chunk_transfer(total, pixel_size, chunk_limit, |chunk, range| {
        let len = range.len();
        let pixels = (len / pixel_size) as u32;
        let set = match chunk {
            Chunk::All => ImageSetHeader::All(size, update),
            Chunk::Start => ImageSetHeader::Start(size, pixels),
            Chunk::Batch => ImageSetHeader::Batch(pixels),
            Chunk::End => ImageSetHeader::End(pixels, update),
        };
        let mut message = header(set, len as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;
        message.reserve_exact(len);
        image.append(&mut message, range.start, len);
        Ok(message)
    })
}

/// Packs an update of the validated `image` into the wire rectangle `rect`
/// (`[x, y, width, height]`) as row strips. `header` serializes an image
/// header with its payload length for the image's state.
pub(crate) fn pack_image_update(
    id: u64,
    image: &ImageData,
    rect: [u32; 4],
    update: bool,
    chunk_limit: usize,
    header: impl Fn(ImageHeader, u32) -> Result<FastVec<32>, ()>,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    let row_bytes = image.row_bytes();
    chunk_rows(
        image.size[0],
        row_bytes,
        chunk_limit,
        update,
        |first, rows, repaint| {
            let strip = [rect[0], rect[1] + first as u32, rect[2], rows as u32];
            let len = rows * row_bytes;
            let mut message = header(
                ImageHeader::Update(strip, image.image_type, repaint),
                len as u32,
            )
            .map_err(|_| format!("Failed to serialize update header for image {}", id))?;
            message.reserve_exact(len);
            image.append(&mut message, first * row_bytes, len);
            Ok(message)
        },
    )
}

pub(crate) fn checked_image_size(size: [usize; 2]) -> Result<[u32; 2], String> {
    let [height, width] = checked_image_shape(size)?;
    if size[0] == 0 || size[1] == 0 {
        return Err("Image dimensions cannot be zero".to_string());
    }

    Ok([width, height])
}

pub(crate) fn checked_image_rect(
    origin: &[usize; 2],
    size: [usize; 2],
) -> Result<[u32; 4], String> {
    let [width, height] = checked_image_size(size)?;
    checked_image_bounds(*origin, size)?;
    let [y, x] = checked_image_shape(*origin)?;

    Ok([x, y, width, height])
}

pub(crate) fn checked_fill_size(size: [usize; 2]) -> Result<([u32; 2], usize), String> {
    let wire_size = checked_image_size(size)?;
    let rgba_size = checked_image_bytes(size, 4)?;

    Ok((wire_size, rgba_size))
}

/// Views `len` bytes at `data` as uninitialized output.
///
/// # Safety
///
/// `data` must point to `len` writable bytes that nothing else accesses.
unsafe fn output<'a>(data: *mut u8, len: usize) -> &'a mut [MaybeUninit<u8>] {
    unsafe { std::slice::from_raw_parts_mut(data.cast::<MaybeUninit<u8>>(), len) }
}

/// Converts `pixel_count` packed source pixels at `data` to RGBA at
/// `new_data`.
///
/// # Safety
///
/// `data` must point to `pixel_count` readable source pixels and `new_data`
/// to `pixel_count * 4` writable bytes, not overlapping them.
#[inline]
pub(crate) unsafe fn write_all_new(
    data: *const u8,
    new_data: *mut u8,
    pixel_count: usize,
    image_type: ImageType,
) {
    let source =
        unsafe { std::slice::from_raw_parts(data, pixel_count * image_type.bytes_per_pixel()) };
    to_rgba(image_type, source, unsafe {
        output(new_data, pixel_count * 4)
    });
}

/// Like [`write_all_new`] for source rows `stride` bytes apart.
///
/// # Safety
///
/// As for [`write_all_new`], with `size[0]` source rows of `size[1]` pixels
/// starting `stride` bytes apart.
#[inline]
pub(super) unsafe fn write_all_new_stride(
    data: *const u8,
    new_data: *mut u8,
    stride: usize,
    size: &[usize; 2],
    image_type: ImageType,
) {
    let row = size[1] * image_type.bytes_per_pixel();
    for i in 0..size[0] {
        let source = unsafe { std::slice::from_raw_parts(data.add(i * stride), row) };
        let target = unsafe { output(new_data.add(i * size[1] * 4), size[1] * 4) };
        to_rgba(image_type, source, target);
    }
}

/// Converts source rows `stride` bytes apart (`0` for packed rows) into the
/// rectangle at `origin` of an RGBA image `old_stride` pixels wide.
///
/// # Safety
///
/// `data` must hold the source rows, and the rectangle must lie inside the
/// RGBA image at `old_data`, which nothing else accesses.
#[inline]
pub(super) unsafe fn write_rectangle(
    data: *const u8,
    stride: usize,
    old_data: *mut u8,
    old_stride: usize,
    origin: &[usize; 2],
    size: &[usize; 2],
    image_type: ImageType,
) {
    let [top, left] = *origin;
    let row = size[1] * image_type.bytes_per_pixel();
    let stride = if stride == 0 { row } else { stride };
    for i in 0..size[0] {
        let source = unsafe { std::slice::from_raw_parts(data.add(i * stride), row) };
        let pixel = (top + i) * old_stride + left;
        let target = unsafe { output(old_data.add(pixel * 4), size[1] * 4) };
        to_rgba(image_type, source, target);
    }
}
