use std::collections::VecDeque;

use std::mem::MaybeUninit;

use crate::protocol::FastVec;
use crate::protocol::image::{
    ImageHeader, ImageSetHeader, ImageType, ImageUpdateHeader, checked_image_bounds,
    checked_image_bytes, checked_image_shape, to_rgba,
};
use crate::server_core::transfer::{Chunk, Messages, chunk_transfer};

/// A borrowed source image in one of the accepted pixel formats.
///
/// `size` is `[height, width]`. Row `i` starts `i * stride` bytes into
/// `data`, which reaches at least to the end of the last row.
#[derive(Clone, Copy)]
pub(crate) struct ImageData<'a> {
    pub size: [usize; 2],
    pub stride: usize,
    pub image_type: ImageType,
    pub data: &'a [u8],
}

impl<'a> ImageData<'a> {
    /// An image whose rows follow each other without gaps.
    pub(crate) fn packed(data: &'a [u8], size: [usize; 2], image_type: ImageType) -> Self {
        Self {
            size,
            stride: size[1] * image_type.bytes_per_pixel(),
            image_type,
            data,
        }
    }

    fn row_bytes(&self) -> usize {
        self.size[1] * self.image_type.bytes_per_pixel()
    }

    pub(crate) fn contiguous(&self) -> bool {
        self.stride == self.row_bytes()
    }

    fn row(&self, row: usize) -> &'a [u8] {
        let start = row * self.stride;
        &self.data[start..start + self.row_bytes()]
    }

    /// Appends `len` pixel bytes starting `start` bytes into the image, as if
    /// its rows were contiguous.
    fn append(&self, message: &mut FastVec<32>, start: usize, len: usize) {
        if self.contiguous() {
            message.extend_from_slice(&self.data[start..start + len]);
            return;
        }

        let row_bytes = self.row_bytes();
        let mut processed = 0;
        while processed < len {
            let offset = start + processed;
            let row_offset = offset % row_bytes;
            let copy = (row_bytes - row_offset).min(len - processed);
            let row = self.row(offset / row_bytes);
            message.extend_from_slice(&row[row_offset..row_offset + copy]);
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

/// Checks the `max_lines` option of an image update.
pub(crate) fn checked_max_lines(max_lines: Option<usize>) -> Result<(), String> {
    match max_lines {
        Some(0) => Err("max_lines must be at least 1".to_string()),
        _ => Ok(()),
    }
}

/// Whether the server rectangle `outer` (`[y, x, height, width]`) contains
/// `inner`.
pub(crate) fn covers(outer: [usize; 4], inner: [usize; 4]) -> bool {
    outer[0] <= inner[0]
        && outer[1] <= inner[1]
        && outer[0] + outer[2] >= inner[0] + inner[2]
        && outer[1] + outer[3] >= inner[1] + inner[3]
}

/// Packs an update of the validated `image` into the wire rectangle `rect`
/// (`[x, y, width, height]`). `header` serializes an image header with its
/// payload length for the image's state.
///
/// The rectangle is cut into sub-rectangles of at most `max_lines` rows, or
/// kept whole without a limit. Each sub-rectangle is one group of messages,
/// chunked like an image set, which the client applies once; `update` asks
/// for a repaint after every group.
pub(crate) fn pack_image_update(
    id: u64,
    image: &ImageData,
    rect: [u32; 4],
    update: bool,
    max_lines: Option<usize>,
    chunk_limit: usize,
    header: impl Fn(ImageHeader, u32) -> Result<FastVec<32>, ()>,
) -> Result<VecDeque<Messages>, String> {
    checked_max_lines(max_lines)?;
    let rows = image.size[0];
    let row_bytes = image.row_bytes();
    let pixel_size = image.image_type.bytes_per_pixel();
    let step = max_lines.unwrap_or(rows).clamp(1, rows.max(1));

    let mut groups = VecDeque::with_capacity(rows.div_ceil(step));
    let mut first = 0;
    while first < rows {
        let count = (rows - first).min(step);
        let part = [rect[0], rect[1] + first as u32, rect[2], count as u32];
        let offset = first * row_bytes;
        let group = chunk_transfer(
            count * row_bytes,
            pixel_size,
            chunk_limit,
            |chunk, range| {
                let len = range.len();
                let pixels = (len / pixel_size) as u32;
                let part_header = match chunk {
                    Chunk::All => ImageUpdateHeader::All(part, update),
                    Chunk::Start => ImageUpdateHeader::Start(part, pixels),
                    Chunk::Batch => ImageUpdateHeader::Batch(pixels),
                    Chunk::End => ImageUpdateHeader::End(pixels, update),
                };
                let mut message = header(
                    ImageHeader::Update(part_header, image.image_type),
                    len as u32,
                )
                .map_err(|_| format!("Failed to serialize update header for image {}", id))?;
                message.reserve_exact(len);
                image.append(&mut message, offset + range.start, len);
                Ok(message)
            },
        )?;
        groups.push_back(group);
        first += count;
    }
    Ok(groups)
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

/// Converts `image` to RGBA into `target`, four bytes per pixel.
fn write_rgba(image: &ImageData, target: &mut [MaybeUninit<u8>]) {
    if image.contiguous() {
        let len = image.size[0] * image.row_bytes();
        to_rgba(image.image_type, &image.data[..len], target);
        return;
    }

    let row = image.size[1] * 4;
    for (i, target) in target.chunks_exact_mut(row).enumerate() {
        to_rgba(image.image_type, image.row(i), target);
    }
}

/// Stores the validated `image` as RGBA in `target`. The allocation is
/// reused unless the image was `resized`, so a smaller image frees memory.
pub(crate) fn store_rgba(target: &mut Vec<u8>, image: &ImageData, resized: bool) {
    let len = image.size[0] * image.size[1] * 4;
    if resized {
        *target = Vec::with_capacity(len);
    } else {
        target.clear();
        target.reserve_exact(len);
    }
    write_rgba(image, &mut target.spare_capacity_mut()[..len]);
    // SAFETY: `write_rgba` initialized the first `len` bytes.
    unsafe { target.set_len(len) };
}

/// Converts the validated `image` to RGBA into the rectangle at `origin`
/// (`[y, x]`) of the RGBA image `target`, which is `width` pixels wide and
/// contains the rectangle.
pub(crate) fn write_rectangle(
    image: &ImageData,
    target: &mut [u8],
    width: usize,
    origin: &[usize; 2],
) {
    let [top, left] = *origin;
    let row = image.size[1] * 4;
    for i in 0..image.size[0] {
        let start = ((top + i) * width + left) * 4;
        let target = &mut target[start..start + row];
        // SAFETY: `MaybeUninit<u8>` has the layout of `u8`, and `to_rgba`
        // writes only initialized bytes.
        let target = unsafe { &mut *(target as *mut [u8] as *mut [MaybeUninit<u8>]) };
        to_rgba(image.image_type, image.row(i), target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two gray rows of two pixels, 3 bytes apart, with no bytes after the
    /// last row.
    fn strided() -> ImageData<'static> {
        ImageData {
            size: [2, 2],
            stride: 3,
            image_type: ImageType::Gray,
            data: &[1, 2, 0, 3, 4],
        }
    }

    #[test]
    fn strided_rows_end_at_the_last_pixel() {
        let image = strided();
        assert!(!image.contiguous());

        let mut message = FastVec::<32>::new();
        image.append(&mut message, 1, 3);
        assert_eq!(message.into_bytes().as_ref(), [2, 3, 4]);

        let mut rgba = Vec::new();
        store_rgba(&mut rgba, &image, true);
        let pixel = |gray| [gray, gray, gray, 255];
        assert_eq!(rgba, [pixel(1), pixel(2), pixel(3), pixel(4)].concat());

        let mut target = vec![0; 3 * 2 * 4];
        write_rectangle(&image, &mut target, 2, &[1, 0]);
        assert_eq!(&target[..8], [0; 8]);
        assert_eq!(&target[8..], rgba);
    }

    #[test]
    fn same_size_reuses_the_allocation() {
        let image = ImageData::packed(&[7; 6], [2, 3], ImageType::Gray);
        let mut rgba = Vec::new();
        store_rgba(&mut rgba, &image, true);
        let allocation = rgba.as_ptr();
        store_rgba(&mut rgba, &image, false);
        assert_eq!(rgba.as_ptr(), allocation);
        assert_eq!(rgba.len(), 24);
    }
}
