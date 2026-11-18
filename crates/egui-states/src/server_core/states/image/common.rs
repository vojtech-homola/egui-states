use std::collections::VecDeque;

use std::mem::MaybeUninit;

use crate::protocol::FastVec;
use crate::protocol::image::{
    ImageHeader, ImageSetHeader, ImageType, ImageUpdateHeader, checked_image_bounds,
    checked_image_bytes, checked_image_shape, to_rgba, widen, widen_into,
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

/// Checks that an image of `size` (`[height, width]`) can be stored and
/// returns its wire size (`[width, height]`). The size is checked at four
/// bytes per pixel whatever the format, because an update can widen the
/// stored image up to RGBA and the client always stores RGBA.
pub(crate) fn checked_stored_size(size: [usize; 2]) -> Result<[u32; 2], String> {
    let wire_size = checked_image_size(size)?;
    checked_image_bytes(size, 4)?;

    Ok(wire_size)
}

/// Converts `image` to `to`, which holds its format, into `target`, as if
/// its rows were contiguous.
fn write_pixels(image: &ImageData, to: ImageType, target: &mut [MaybeUninit<u8>]) {
    if image.contiguous() {
        let len = image.size[0] * image.row_bytes();
        widen(image.image_type, to, &image.data[..len], target);
        return;
    }

    let row = image.size[1] * to.bytes_per_pixel();
    for (i, target) in target.chunks_exact_mut(row).enumerate() {
        widen(image.image_type, to, image.row(i), target);
    }
}

/// Replaces `target` with `len` bytes written by `write`. The allocation is
/// reused for the same length, so a smaller image frees memory.
fn replace_bytes(target: &mut Vec<u8>, len: usize, write: impl FnOnce(&mut [MaybeUninit<u8>])) {
    if target.len() == len {
        target.clear();
    } else {
        *target = Vec::with_capacity(len);
    }
    write(&mut target.spare_capacity_mut()[..len]);
    // SAFETY: `write` initialized the first `len` bytes.
    unsafe { target.set_len(len) };
}

/// The color of a fill, as the pixel of the format the image is then stored
/// in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageFill {
    Gray([u8; 1]),
    GrayAlpha([u8; 2]),
    Color([u8; 3]),
    ColorAlpha([u8; 4]),
}

impl ImageFill {
    pub(crate) fn image_type(self) -> ImageType {
        match self {
            Self::Gray(_) => ImageType::Gray,
            Self::GrayAlpha(_) => ImageType::GrayAlpha,
            Self::Color(_) => ImageType::Color,
            Self::ColorAlpha(_) => ImageType::ColorAlpha,
        }
    }

    /// The color the client fills its texture with: the stored pixel
    /// expanded to RGBA, so the two cannot differ.
    pub(crate) fn rgba(self) -> [u8; 4] {
        let pixel: &[u8] = match &self {
            Self::Gray(pixel) => pixel,
            Self::GrayAlpha(pixel) => pixel,
            Self::Color(pixel) => pixel,
            Self::ColorAlpha(pixel) => pixel,
        };
        let mut rgba = [MaybeUninit::uninit(); 4];
        to_rgba(self.image_type(), pixel, &mut rgba);
        // SAFETY: `to_rgba` wrote all four bytes.
        rgba.map(|byte| unsafe { byte.assume_init() })
    }
}

/// A server image: its pixels in the format of the last set or fill,
/// widened by updates with more channels.
pub(crate) struct StoredImage {
    data: Vec<u8>,
    /// `[height, width]`.
    size: [usize; 2],
    image_type: ImageType,
}

impl StoredImage {
    /// Stores the validated `image` in its own format.
    pub(crate) fn new(image: &ImageData) -> Self {
        let mut stored = Self {
            data: Vec::new(),
            size: image.size,
            image_type: image.image_type,
        };
        stored.set(image);
        stored
    }

    /// An image of `size` filled with `fill`, in its format.
    pub(crate) fn filled(size: [usize; 2], fill: ImageFill) -> Self {
        let mut stored = Self {
            data: Vec::new(),
            size,
            image_type: fill.image_type(),
        };
        stored.fill(size, fill);
        stored
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn size(&self) -> [usize; 2] {
        self.size
    }

    pub(crate) fn image_type(&self) -> ImageType {
        self.image_type
    }

    /// The stored pixels, for packing a whole-image set.
    pub(crate) fn image_data(&self) -> ImageData<'_> {
        ImageData::packed(&self.data, self.size, self.image_type)
    }

    /// Replaces the image with the validated `image`, in its format.
    pub(crate) fn set(&mut self, image: &ImageData) {
        let len = image.size[0] * image.row_bytes();
        replace_bytes(&mut self.data, len, |target| {
            write_pixels(image, image.image_type, target)
        });
        self.size = image.size;
        self.image_type = image.image_type;
    }

    /// Fills the image with `fill`, in its format.
    pub(crate) fn fill(&mut self, size: [usize; 2], fill: ImageFill) {
        fn repeat<const N: usize>(target: &mut [MaybeUninit<u8>], pixel: [u8; N]) {
            for target in target.as_chunks_mut::<N>().0 {
                *target = pixel.map(MaybeUninit::new);
            }
        }

        let image_type = fill.image_type();
        let len = size[0] * size[1] * image_type.bytes_per_pixel();
        replace_bytes(&mut self.data, len, |target| match fill {
            ImageFill::Gray(pixel) => repeat(target, pixel),
            ImageFill::GrayAlpha(pixel) => repeat(target, pixel),
            ImageFill::Color(pixel) => repeat(target, pixel),
            ImageFill::ColorAlpha(pixel) => repeat(target, pixel),
        });
        self.size = size;
        self.image_type = image_type;
    }

    /// Whether the rectangle of `size` at `origin` (`[y, x]`) lies inside.
    pub(crate) fn contains(&self, origin: &[usize; 2], size: [usize; 2]) -> bool {
        let inside = |axis: usize| {
            origin[axis]
                .checked_add(size[axis])
                .is_some_and(|end| end <= self.size[axis])
        };
        inside(0) && inside(1)
    }

    /// Writes the validated `image` at `origin` (`[y, x]`), which
    /// [`contains`](Self::contains) it. With channels the stored image lacks,
    /// the whole image is widened first, so it holds both formats.
    pub(crate) fn update(&mut self, image: &ImageData, origin: &[usize; 2]) {
        let wider = self.image_type.union(image.image_type);
        if wider != self.image_type {
            let from = self.image_type;
            let mut data = Vec::new();
            let len = self.size[0] * self.size[1] * wider.bytes_per_pixel();
            replace_bytes(&mut data, len, |target| {
                widen(from, wider, &self.data, target)
            });
            self.data = data;
            self.image_type = wider;
        }

        let pixel_size = self.image_type.bytes_per_pixel();
        let [top, left] = *origin;
        let row = image.size[1] * pixel_size;
        for i in 0..image.size[0] {
            let start = ((top + i) * self.size[1] + left) * pixel_size;
            let target = &mut self.data[start..start + row];
            widen_into(image.image_type, self.image_type, image.row(i), target);
        }
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

        let stored = StoredImage::new(&image);
        assert_eq!(stored.data(), [1, 2, 3, 4]);
        assert_eq!(stored.image_type(), ImageType::Gray);

        let mut target = StoredImage::filled([3, 2], ImageFill::Gray([0]));
        target.update(&image, &[1, 0]);
        assert_eq!(target.data(), [0, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn the_same_byte_length_reuses_the_allocation() {
        let image = ImageData::packed(&[7; 6], [2, 3], ImageType::Gray);
        let mut stored = StoredImage::new(&image);
        let allocation = stored.data().as_ptr();
        stored.set(&image);
        assert_eq!(stored.data().as_ptr(), allocation);
        // Three gray-alpha pixels hold the six bytes of six gray ones.
        stored.fill([1, 3], ImageFill::GrayAlpha([5, 8]));
        assert_eq!(stored.data().as_ptr(), allocation);
        assert_eq!(stored.data(), [5, 8, 5, 8, 5, 8]);
        stored.set(&ImageData::packed(&[1; 3], [1, 3], ImageType::Gray));
        assert_eq!(stored.data(), [1; 3]);
    }

    #[test]
    fn a_fill_is_stored_in_its_format_and_sent_as_the_same_color() {
        for (fill, image_type, pixel, rgba) in [
            (
                ImageFill::Gray([1]),
                ImageType::Gray,
                &[1][..],
                [1, 1, 1, 255],
            ),
            (
                ImageFill::GrayAlpha([1, 4]),
                ImageType::GrayAlpha,
                &[1, 4],
                [1, 1, 1, 4],
            ),
            (
                ImageFill::Color([1, 2, 3]),
                ImageType::Color,
                &[1, 2, 3],
                [1, 2, 3, 255],
            ),
            (
                ImageFill::ColorAlpha([1, 2, 3, 4]),
                ImageType::ColorAlpha,
                &[1, 2, 3, 4],
                [1, 2, 3, 4],
            ),
        ] {
            let stored = StoredImage::filled([2, 1], fill);
            assert_eq!(stored.image_type(), image_type);
            assert_eq!(stored.data(), pixel.repeat(2));
            assert_eq!(fill.rgba(), rgba);
        }
    }

    #[test]
    fn an_update_with_fewer_channels_keeps_the_stored_format() {
        let mut stored = StoredImage::filled([1, 2], ImageFill::ColorAlpha([1, 2, 3, 4]));
        stored.update(&ImageData::packed(&[9], [1, 1], ImageType::Gray), &[0, 1]);
        assert_eq!(stored.image_type(), ImageType::ColorAlpha);
        assert_eq!(stored.data(), [1, 2, 3, 4, 9, 9, 9, 255]);
    }

    #[test]
    fn an_update_with_more_channels_widens_the_whole_image_first() {
        let mut stored = StoredImage::filled([1, 2], ImageFill::Gray([7]));
        stored.update(
            &ImageData::packed(&[1, 2, 3], [1, 1], ImageType::Color),
            &[0, 1],
        );
        assert_eq!(stored.image_type(), ImageType::Color);
        assert_eq!(stored.data(), [7, 7, 7, 1, 2, 3]);

        // Gray-alpha and color meet at color-alpha.
        stored.update(
            &ImageData::packed(&[5, 6], [1, 1], ImageType::GrayAlpha),
            &[0, 0],
        );
        assert_eq!(stored.image_type(), ImageType::ColorAlpha);
        assert_eq!(stored.data(), [5, 5, 5, 6, 1, 2, 3, 255]);

        // Only a set narrows it again.
        stored.set(&ImageData::packed(&[4, 4], [1, 2], ImageType::Gray));
        assert_eq!(stored.image_type(), ImageType::Gray);
        assert_eq!(stored.data(), [4, 4]);
    }

    #[test]
    fn contains_checks_both_axes_and_overflow() {
        let stored = StoredImage::filled([2, 3], ImageFill::Gray([0]));
        assert!(stored.contains(&[0, 0], [2, 3]));
        assert!(stored.contains(&[1, 2], [1, 1]));
        assert!(!stored.contains(&[1, 0], [2, 1]));
        assert!(!stored.contains(&[0, 3], [1, 1]));
        assert!(!stored.contains(&[usize::MAX, 0], [1, 1]));
    }
}
