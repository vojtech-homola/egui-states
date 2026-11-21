//! Server-controlled image storage and image-format helpers.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::protocol::image::{ImageType, checked_image_bytes};
use crate::server_core::states::image::common::{ImageFill, StoredImage, checked_stored_size};
use crate::server_core::states::image::multi::ImageMulti as CoreImageMulti;
use crate::server_core::states::image::single::{Image as CoreImage, ImageData};

use super::data::{HeaderValue, decode_header, header_type};
use super::state_server::{StateServer, serialize_bytes};
use super::{Result, ServerError};

/// A uniform image color for [`Image::set_all`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageColor {
    /// Opaque grayscale intensity.
    Gray(u8),
    /// Grayscale intensity and alpha.
    GrayAlpha(u8, u8),
    /// Opaque red, green, and blue channels.
    Color(u8, u8, u8),
    /// Red, green, blue, and alpha channels.
    ColorAlpha(u8, u8, u8, u8),
}

impl ImageColor {
    /// The fill of this color, in the format it is stored in.
    #[inline]
    fn fill(self) -> ImageFill {
        match self {
            Self::Gray(gray) => ImageFill::Gray([gray]),
            Self::GrayAlpha(gray, alpha) => ImageFill::GrayAlpha([gray, alpha]),
            Self::Color(red, green, blue) => ImageFill::Color([red, green, blue]),
            Self::ColorAlpha(red, green, blue, alpha) => {
                ImageFill::ColorAlpha([red, green, blue, alpha])
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Channel layout of image bytes passed to [`Image::set`] or [`Image::update`],
/// and of the image the server stores, as [`Image::get`] returns it. The
/// client expands every format to its four-channel egui texture storage.
pub enum ImageFormat {
    /// Three bytes per pixel: RGB.
    Color,
    /// Four bytes per pixel: RGBA.
    ColorAlpha,
    /// One byte per pixel: grayscale intensity.
    Gray,
    /// Two bytes per pixel: grayscale intensity and alpha.
    GrayAlpha,
}

impl ImageFormat {
    fn image_type(self) -> ImageType {
        match self {
            Self::Color => ImageType::Color,
            Self::ColorAlpha => ImageType::ColorAlpha,
            Self::Gray => ImageType::Gray,
            Self::GrayAlpha => ImageType::GrayAlpha,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        self.image_type().bytes_per_pixel()
    }

    fn from_image_type(image_type: ImageType) -> Self {
        match image_type {
            ImageType::Color => Self::Color,
            ImageType::ColorAlpha => Self::ColorAlpha,
            ImageType::Gray => Self::Gray,
            ImageType::GrayAlpha => Self::GrayAlpha,
        }
    }
}

/// A copy of a stored image and its header, as `get_with_header` returns it:
/// pixels, `[height, width]` shape, format and header.
type ImageWithHeader<H> = (Vec<u8>, [usize; 2], ImageFormat, H);

/// A copy of a stored image: its pixels, `[height, width]` shape and format.
fn copy_stored(image: &StoredImage) -> (Vec<u8>, [usize; 2], ImageFormat) {
    (
        image.data().to_vec(),
        image.size(),
        ImageFormat::from_image_type(image.image_type()),
    )
}

/// Server-controlled image mirrored to the client, with an optional typed
/// header `H` that describes it, `()` (the default) for an image without one.
///
/// The server keeps the image in the format of the last [`set`](Self::set)
/// or [`set_all`](Self::set_all). An [`update`](Self::update) with channels
/// the image lacks widens it first, to the smallest format that holds both:
/// gray gains color channels equal to it, and a missing alpha becomes 255,
/// which is what the client shows. Until it is first set, the image has no
/// format and no pixels.
///
/// With a header type, the image never exists without a header: the set that
/// creates it must carry one ([`set_with_header`](Self::set_with_header) or
/// [`set_all_with_header`](Self::set_all_with_header)). After that, every
/// change can carry a new header, and a change without one keeps it. The
/// client receives the header with the pixels it belongs to.
///
/// Each dimension is limited to 65,536 pixels, inclusive. Client renderer and
/// memory limits may be smaller.
pub struct Image<H = ()> {
    inner: Arc<CoreImage>,
    _header: PhantomData<fn() -> H>,
}

impl<H> Clone for Image<H> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _header: PhantomData,
        }
    }
}

impl<H: HeaderValue> Image<H> {
    /// Registers an empty image under `name`.
    ///
    /// With `low_priority`, its transfers use the low-priority lane: they are
    /// sent only while no other state has a message waiting, and only a
    /// window of their data is on the way at a time. Small states then do
    /// not wait behind its transfers, but a change of another state made
    /// after a change of this one can reach the client first.
    pub fn new(server: &StateServer, name: impl Into<String>, low_priority: bool) -> Result<Self> {
        let (_, inner) = server.add_image(name.into(), header_type::<H>(), low_priority)?;
        Ok(Self {
            inner,
            _header: PhantomData,
        })
    }

    /// Returns the image shape as `[height, width]`, `[0, 0]` until the
    /// image is set.
    pub fn shape(&self) -> [usize; 2] {
        self.inner.get_size()
    }

    /// Returns the format the image is stored in, `None` until it is set.
    pub fn format(&self) -> Option<ImageFormat> {
        self.inner
            .get_image(|image| image.map(|image| ImageFormat::from_image_type(image.image_type())))
    }

    /// Returns a copy of the pixels, the `[height, width]` shape and the
    /// format they are stored in, `None` until the image is set.
    pub fn get(&self) -> Option<(Vec<u8>, [usize; 2], ImageFormat)> {
        self.inner.get_image(|image| image.map(copy_stored))
    }

    /// Returns the header, or `None` until the image is set.
    pub fn header(&self) -> Result<Option<H>> {
        self.inner
            .get_image(|image| image.map(|image| decode_header(image.header())).transpose())
    }

    /// Returns a copy of the image, as [`Self::get`] does, and the header
    /// that belongs to it, read together, or `None` until the image is set.
    pub fn get_with_header(&self) -> Result<Option<ImageWithHeader<H>>> {
        self.inner.get_image(|image| {
            image
                .map(|image| {
                    let (data, shape, format) = copy_stored(image);
                    Ok((data, shape, format, decode_header(image.header())?))
                })
                .transpose()
        })
    }

    /// Returns whether nothing is being sent and nothing waits, so the next
    /// [`set`](Self::set), [`set_all`](Self::set_all) or
    /// [`update`](Self::update) is sent without waiting.
    ///
    /// Also true while no client is connected, because a change is then only
    /// stored. Producers that want the freshest image on a slow link can skip
    /// a frame while this is false.
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    /// Replaces the complete image, keeping its header.
    ///
    /// `size` is `[height, width]`; `data` must contain exactly the number of
    /// bytes implied by `size` and `format`, which the server keeps it in.
    /// The client shows the new image only once all of it has arrived.
    ///
    /// One transfer of this image is sent at a time: the call blocks until
    /// the previous one has been acknowledged by the client, then queues the
    /// image and returns. The unsent parts of a split [`update`](Self::update)
    /// are dropped first, because this image overwrites them. With `wait`,
    /// the call returns only once the client has acknowledged this image, or
    /// the connection has ended; it does not tell the two apart, and without
    /// a client it does not wait.
    ///
    /// # Errors
    ///
    /// Returns an error if a dimension is zero or exceeds 65,536 pixels,
    /// dimensions overflow, the data length does not match `size` and `format`,
    /// the image is not set yet and the state has a header type (use
    /// [`Self::set_with_header`] to create it), or the update cannot be
    /// queued.
    pub fn set(
        &self,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        wait: bool,
    ) -> Result<()> {
        self.set_inner(data, size, format, None, wait)
    }

    /// Like [`Self::set`], replacing the header as well.
    ///
    /// # Errors
    ///
    /// As [`Self::set`], and when the state has no header type or the header
    /// is larger than a value may be.
    pub fn set_with_header(
        &self,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        header: &H,
        wait: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.set_inner(data, size, format, Some(&header), wait)
    }

    fn set_inner(
        &self,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        header: Option<&[u8]>,
        wait: bool,
    ) -> Result<()> {
        check_image_data(data, size, format)?;
        let image = ImageData::packed(data, size, format.image_type());
        self.inner
            .set_image(image, header, wait)
            .map_err(ServerError::new)
    }

    /// Fill an image with one color, keeping its header.
    ///
    /// `shape` is `[height, width]`. The server keeps the image in the format
    /// of `color`: [`ImageColor::Gray`] stores one channel, and so on. When a
    /// client is connected, this sends only a compact header; the server
    /// still retains the complete image for subsequent reads, updates, and
    /// connection synchronization. It is scheduled like [`set`](Self::set),
    /// including `wait`.
    ///
    /// # Errors
    ///
    /// Returns an error for zero dimensions, dimensions exceeding 65,536 pixels,
    /// overflowing dimensions, an image that is not set yet in a state with a
    /// header type, or when the update cannot be queued.
    pub fn set_all(&self, shape: [usize; 2], color: ImageColor, wait: bool) -> Result<()> {
        self.inner
            .set_all_image(shape, color.fill(), None, wait)
            .map_err(ServerError::new)
    }

    /// Like [`Self::set_all`], replacing the header as well.
    ///
    /// # Errors
    ///
    /// As [`Self::set_all`], and as [`Self::set_with_header`] for the
    /// header.
    pub fn set_all_with_header(
        &self,
        shape: [usize; 2],
        color: ImageColor,
        header: &H,
        wait: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set_all_image(shape, color.fill(), Some(&header), wait)
            .map_err(ServerError::new)
    }

    /// Replaces a rectangular image region, keeping the header.
    ///
    /// `origin` and `size` are `[y, x]` and `[height, width]`. When `format`
    /// has channels the stored image lacks, the server first widens the whole
    /// image to a format that holds both, as described at [`Image`]. Without
    /// `max_lines`, the client applies the whole rectangle at once. With
    /// `max_lines`, the rectangle is sent as parts of at most that many rows
    /// from the top; the client applies and shows each part as it arrives.
    ///
    /// Like [`set`](Self::set), the call blocks until the previous transfer
    /// has been acknowledged, then queues the update and returns. A split
    /// update queues only its first part; each further part is sent when the
    /// client has acknowledged the one before, so later changes of other
    /// states can reach the client before them, and the next call of this
    /// image waits until the last part has been acknowledged. With `force`,
    /// the call first drops the unsent parts of the previous split update
    /// when this rectangle covers its rectangle; the part being sent always
    /// completes. With `wait`, the call returns only once the client has
    /// acknowledged the last part of this update, or the connection has
    /// ended.
    ///
    /// # Errors
    ///
    /// Returns an error if the rectangle lies outside the current image, a
    /// dimension is zero, its dimensions exceed 65,536 pixels or overflow, its
    /// data length is invalid, `max_lines` is zero, or it cannot be queued.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<()> {
        self.update_inner(data, origin, size, format, None, force, max_lines, wait)
    }

    /// Like [`Self::update`], replacing the header as well. A split update
    /// carries the header with its first part, and the client shows it
    /// together with that part.
    ///
    /// # Errors
    ///
    /// As [`Self::update`], and as [`Self::set_with_header`] for the header.
    #[allow(clippy::too_many_arguments)]
    pub fn update_with_header(
        &self,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        header: &H,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.update_inner(
            data,
            origin,
            size,
            format,
            Some(&header),
            force,
            max_lines,
            wait,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn update_inner(
        &self,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        header: Option<&[u8]>,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<()> {
        check_image_data(data, size, format)?;
        let image = ImageData::packed(data, size, format.image_type());
        self.inner
            .update_image(&origin, image, header, force, max_lines, wait)
            .map_err(ServerError::new)
    }

    /// Replaces the header alone.
    ///
    /// The client does not acknowledge a header, so this does not wait for
    /// the client itself. It is sent after the transfers of this image that
    /// are already queued, including the unsent parts of a split update, and
    /// waits until the client has acknowledged them.
    ///
    /// # Errors
    ///
    /// Returns an error when the image is not set, the state has no header
    /// type, or the header is larger than a value may be.
    pub fn set_header(&self, header: &H) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner.set_header(&header).map_err(ServerError::new)
    }
}

fn check_image_data(data: &[u8], size: [usize; 2], format: ImageFormat) -> Result<()> {
    checked_image_bytes(size, 4).map_err(ServerError::new)?;
    let stride = size[1]
        .checked_mul(format.bytes_per_pixel())
        .ok_or_else(|| ServerError::new("image dimensions overflow"))?;
    let expected = size[0]
        .checked_mul(stride)
        .ok_or_else(|| ServerError::new("image dimensions overflow"))?;
    if data.len() != expected {
        return Err(ServerError::new(format!(
            "invalid image data size: expected {expected}, got {}",
            data.len()
        )));
    }
    Ok(())
}

/// Server-controlled images indexed by sparse `u32` keys, each with an
/// optional typed header `H`, `()` (the default) for images without one.
/// Each image keeps its own format and header, as described at [`Image`];
/// creating an index of a state with a header type needs a header.
///
/// Each image dimension is limited to 65,536 pixels, inclusive. Client renderer
/// and memory limits may be smaller.
pub struct ImageMulti<H = ()> {
    inner: Arc<CoreImageMulti>,
    _header: PhantomData<fn() -> H>,
}

impl<H> Clone for ImageMulti<H> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _header: PhantomData,
        }
    }
}

impl<H: HeaderValue> ImageMulti<H> {
    /// Registers an empty keyed image collection under `name`. With
    /// `low_priority`, its transfers use the low-priority lane, as described
    /// at [`Image::new`].
    pub fn new(server: &StateServer, name: impl Into<String>, low_priority: bool) -> Result<Self> {
        let (_, inner) = server.add_image_multi(name.into(), header_type::<H>(), low_priority)?;
        Ok(Self {
            inner,
            _header: PhantomData,
        })
    }

    /// Returns the image shape at `index` as `[height, width]`.
    pub fn shape(&self, index: u32) -> Option<[usize; 2]> {
        self.inner.get_size(index)
    }

    /// Returns the format the image at `index` is stored in.
    pub fn format(&self, index: u32) -> Option<ImageFormat> {
        self.inner.get_image(index, |image| {
            image.map(|image| ImageFormat::from_image_type(image.image_type()))
        })
    }

    /// Returns a copy of the pixels, the `[height, width]` shape and the
    /// format of the image at `index`.
    pub fn get(&self, index: u32) -> Option<(Vec<u8>, [usize; 2], ImageFormat)> {
        self.inner.get_image(index, |image| image.map(copy_stored))
    }

    /// Returns the header of the image at `index`, `None` for a missing
    /// index.
    pub fn header(&self, index: u32) -> Result<Option<H>> {
        self.inner.get_image(index, |image| {
            image.map(|image| decode_header(image.header())).transpose()
        })
    }

    /// Returns a copy of the image at `index` and its header, read together,
    /// as [`Image::get_with_header`] does.
    pub fn get_with_header(&self, index: u32) -> Result<Option<ImageWithHeader<H>>> {
        self.inner.get_image(index, |image| {
            image
                .map(|image| {
                    let (data, shape, format) = copy_stored(image);
                    Ok((data, shape, format, decode_header(image.header())?))
                })
                .transpose()
        })
    }

    /// Returns the number of populated image keys.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the collection contains no images.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Returns whether `index` is populated.
    pub fn contains(&self, index: u32) -> bool {
        self.inner.contains(index)
    }

    /// Returns populated indices in ascending order.
    pub fn indices(&self) -> Vec<u32> {
        self.inner.indices()
    }

    /// Returns whether nothing is being sent and nothing waits for any
    /// index, as [`Image::is_idle`].
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    /// Returns whether nothing of `index` is being sent and nothing of it
    /// waits, so its next change is sent at once.
    pub fn is_index_idle(&self, index: u32) -> bool {
        self.inner.is_index_idle(index)
    }

    /// Creates or replaces the complete image at `index`, keeping its
    /// header.
    ///
    /// Each index has its own transfer: this waits for the previous transfer
    /// of `index` only, as [`Image::set`] does. Only unsent parts of a
    /// split update of the same `index` are dropped. With `wait`, the call
    /// returns only once the client has acknowledged this image, or the
    /// connection has ended.
    ///
    /// # Errors
    ///
    /// As [`Image::set`]; creating an index of a state with a header type
    /// needs [`Self::set_with_header`].
    pub fn set(
        &self,
        index: u32,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        wait: bool,
    ) -> Result<()> {
        self.set_inner(index, data, size, format, None, wait)
    }

    /// Like [`Self::set`], replacing the header as well.
    ///
    /// # Errors
    ///
    /// As [`Image::set_with_header`].
    pub fn set_with_header(
        &self,
        index: u32,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        header: &H,
        wait: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.set_inner(index, data, size, format, Some(&header), wait)
    }

    fn set_inner(
        &self,
        index: u32,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        header: Option<&[u8]>,
        wait: bool,
    ) -> Result<()> {
        check_image_multi_data(data, size, format)?;
        let image = ImageData::packed(data, size, format.image_type());
        self.inner
            .set_image(index, image, header, wait)
            .map_err(ServerError::new)
    }

    /// Creates or replaces the image at `index` with one uniform color, stored
    /// in the format of `color` as [`Image::set_all`] does, keeping its
    /// header. It is scheduled like [`set`](Self::set), including `wait`.
    pub fn set_all(
        &self,
        index: u32,
        shape: [usize; 2],
        color: ImageColor,
        wait: bool,
    ) -> Result<()> {
        self.inner
            .set_all_image(index, shape, color.fill(), None, wait)
            .map_err(ServerError::new)
    }

    /// Like [`Self::set_all`], replacing the header as well.
    pub fn set_all_with_header(
        &self,
        index: u32,
        shape: [usize; 2],
        color: ImageColor,
        header: &H,
        wait: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set_all_image(index, shape, color.fill(), Some(&header), wait)
            .map_err(ServerError::new)
    }

    /// Replaces a rectangular region of the existing image at `index`,
    /// keeping its header.
    ///
    /// `max_lines`, `force` and `wait` work as in [`Image::update`]. With
    /// `force`, waiting parts are dropped only when they belong to the same
    /// `index` and this rectangle covers their rectangle; parts of other
    /// indices are never dropped.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        index: u32,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<()> {
        self.update_inner(
            index, data, origin, size, format, None, force, max_lines, wait,
        )
    }

    /// Like [`Self::update`], replacing the header as well, as
    /// [`Image::update_with_header`] does.
    #[allow(clippy::too_many_arguments)]
    pub fn update_with_header(
        &self,
        index: u32,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        header: &H,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.update_inner(
            index,
            data,
            origin,
            size,
            format,
            Some(&header),
            force,
            max_lines,
            wait,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn update_inner(
        &self,
        index: u32,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        header: Option<&[u8]>,
        force: bool,
        max_lines: Option<usize>,
        wait: bool,
    ) -> Result<()> {
        check_image_multi_data(data, size, format)?;
        let image = ImageData::packed(data, size, format.image_type());
        self.inner
            .update_image(index, &origin, image, header, force, max_lines, wait)
            .map_err(ServerError::new)
    }

    /// Replaces the header of the image at `index` alone, as
    /// [`Image::set_header`] does; it waits only for transfers of `index`.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is missing, the state has no header
    /// type, or the header is larger than a value may be.
    pub fn set_header(&self, index: u32, header: &H) -> Result<()> {
        let header = serialize_bytes(header)?;
        self.inner
            .set_header(index, &header)
            .map_err(ServerError::new)
    }

    /// Removes `index`; an absent index is a no-op.
    ///
    /// Unsent parts of a split update of `index` are dropped, then the call
    /// waits until nothing is in flight before it queues the removal.
    pub fn remove_index(&self, index: u32) -> Result<()> {
        self.inner.remove_index(index).map_err(ServerError::new)
    }

    /// Removes all image keys.
    ///
    /// Unsent parts of a split update are dropped, then the call waits until
    /// nothing is in flight before it queues the reset.
    pub fn reset(&self) -> Result<()> {
        self.inner.reset_images().map_err(ServerError::new)
    }
}

fn check_image_multi_data(data: &[u8], size: [usize; 2], format: ImageFormat) -> Result<()> {
    checked_stored_size(size).map_err(ServerError::new)?;
    let stride = size[1]
        .checked_mul(format.bytes_per_pixel())
        .ok_or_else(|| ServerError::new("image dimensions overflow"))?;
    let expected = size[0]
        .checked_mul(stride)
        .ok_or_else(|| ServerError::new("image dimensions overflow"))?;
    if data.len() != expected {
        return Err(ServerError::new(format!(
            "invalid image data size: expected {expected}, got {}",
            data.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_images_enforce_the_inclusive_shape_limit() {
        let server = StateServer::new().unwrap();
        let image = Image::<()>::new(&server, "root.image", false).unwrap();
        let images = ImageMulti::<()>::new(&server, "root.images", false).unwrap();
        for shape in [[65_536, 1], [1, 65_536]] {
            let data = vec![7; 65_536];
            image.set(&data, shape, ImageFormat::Gray, false).unwrap();
            images
                .set(7, &data, shape, ImageFormat::Gray, false)
                .unwrap();
            image.set_all(shape, ImageColor::Gray(9), false).unwrap();
            images
                .set_all(7, shape, ImageColor::Gray(9), false)
                .unwrap();
            let origin = [shape[0] - 1, shape[1] - 1];
            image
                .update(&[8], origin, [1, 1], ImageFormat::Gray, false, None, false)
                .unwrap();
            images
                .update(
                    7,
                    &[8],
                    origin,
                    [1, 1],
                    ImageFormat::Gray,
                    false,
                    None,
                    false,
                )
                .unwrap();
            assert_eq!(image.shape(), shape);
            assert_eq!(images.shape(7), Some(shape));
            let before = image.get();
            for oversized in [[65_537, 1], [1, 65_537]] {
                for result in [
                    image.set(&[], oversized, ImageFormat::Gray, false),
                    images.set(9, &[], oversized, ImageFormat::Gray, false),
                    image.set_all(oversized, ImageColor::Gray(0), false),
                    images.set_all(9, oversized, ImageColor::Gray(0), false),
                    image.update(
                        &[],
                        [0, 0],
                        oversized,
                        ImageFormat::Gray,
                        false,
                        None,
                        false,
                    ),
                    images.update(
                        7,
                        &[],
                        [0, 0],
                        oversized,
                        ImageFormat::Gray,
                        false,
                        None,
                        false,
                    ),
                ] {
                    assert!(result.unwrap_err().to_string().contains("65536"));
                }
            }
            for origin in [[65_536, 0], [0, 65_536], [usize::MAX, 0], [0, usize::MAX]] {
                assert!(
                    image
                        .update(&[0], origin, [1, 1], ImageFormat::Gray, false, None, false)
                        .is_err()
                );
                assert!(
                    images
                        .update(
                            7,
                            &[0],
                            origin,
                            [1, 1],
                            ImageFormat::Gray,
                            false,
                            None,
                            false
                        )
                        .is_err()
                );
            }
            assert_eq!(image.get(), before);
            assert_eq!(images.get(7), before);
            assert!(!images.contains(9));
        }
    }

    #[test]
    fn image_color_normalizes_to_rgba() {
        assert_eq!(ImageColor::Gray(1).fill().rgba(), [1, 1, 1, 255]);
        assert_eq!(ImageColor::GrayAlpha(1, 2).fill().rgba(), [1, 1, 1, 2]);
        assert_eq!(ImageColor::Color(1, 2, 3).fill().rgba(), [1, 2, 3, 255]);
        assert_eq!(
            ImageColor::ColorAlpha(1, 2, 3, 4).fill().rgba(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn image_keeps_its_format_and_widens_for_an_update() {
        let server = StateServer::new().unwrap();
        let image = Image::<()>::new(&server, "root.image", false).unwrap();
        assert_eq!(image.get(), None);
        assert_eq!(image.format(), None);

        image
            .set(&[10, 20, 30, 40, 50, 60], [1, 2], ImageFormat::Color, false)
            .unwrap();
        assert_eq!(image.shape(), [1, 2]);
        assert_eq!(
            image.get(),
            Some((vec![10, 20, 30, 40, 50, 60], [1, 2], ImageFormat::Color))
        );

        image
            .update(
                &[7, 8],
                [0, 0],
                [1, 1],
                ImageFormat::GrayAlpha,
                false,
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            image.get(),
            Some((
                vec![7, 7, 7, 8, 40, 50, 60, 255],
                [1, 2],
                ImageFormat::ColorAlpha
            ))
        );
        assert_eq!(image.format(), Some(ImageFormat::ColorAlpha));
    }

    #[test]
    fn image_rejects_invalid_and_overflowing_dimensions() {
        let server = StateServer::new().unwrap();
        let image = Image::<()>::new(&server, "root.image", false).unwrap();

        assert!(
            image
                .set(&[], [1, 1], ImageFormat::ColorAlpha, false)
                .is_err()
        );
        assert!(
            image
                .set(&[], [usize::MAX, 2], ImageFormat::ColorAlpha, false)
                .is_err()
        );
    }

    /// The bounds check guards an unchecked pixel copy, so every rejected
    /// rectangle must leave the stored image untouched.
    #[test]
    fn image_update_outside_the_image_is_rejected_without_changes() {
        let server = StateServer::new().unwrap();
        let image = Image::<()>::new(&server, "root.image", false).unwrap();

        // Nothing has been set yet, so every rectangle is outside the image.
        assert!(
            image
                .update(&[1], [0, 0], [1, 1], ImageFormat::Gray, false, None, false)
                .is_err()
        );
        assert_eq!(image.shape(), [0, 0]);

        image
            .set(&[1, 2, 3, 4, 5, 6], [2, 3], ImageFormat::Gray, false)
            .unwrap();
        let before = image.get();
        for (origin, size) in [
            ([0, 0], [3, 1]),
            ([0, 0], [1, 4]),
            ([2, 0], [1, 1]),
            ([0, 3], [1, 1]),
            ([1, 2], [1, 2]),
            ([usize::MAX, 0], [1, 1]),
            ([0, usize::MAX], [1, 1]),
        ] {
            let data = vec![9; size[0] * size[1]];
            assert!(
                image
                    .update(&data, origin, size, ImageFormat::Gray, false, None, false)
                    .is_err(),
                "accepted {size:?} at {origin:?}"
            );
            assert_eq!(
                image.get(),
                before,
                "rejected update at {origin:?} changed pixels"
            );
        }

        // The last row and column are still inside.
        image
            .update(&[9], [1, 2], [1, 1], ImageFormat::Gray, false, None, false)
            .unwrap();
        assert_eq!(image.get().unwrap().0, [1, 2, 3, 4, 5, 9]);
    }

    #[test]
    fn image_set_all_stores_the_format_of_its_color() {
        let server = StateServer::new().unwrap();
        let image = Image::<()>::new(&server, "root.image", false).unwrap();

        image
            .set_all([2, 3], ImageColor::GrayAlpha(7, 8), false)
            .unwrap();

        assert_eq!(image.shape(), [2, 3]);
        assert_eq!(
            image.get(),
            Some(([7, 8].repeat(6), [2, 3], ImageFormat::GrayAlpha))
        );
        assert!(image.set_all([0, 3], ImageColor::Gray(0), false).is_err());
    }

    #[test]
    fn image_multi_sparse_operations_and_validation() {
        let server = StateServer::new().unwrap();
        let images = ImageMulti::<()>::new(&server, "root.images", false).unwrap();

        assert!(images.is_empty());
        assert_eq!(images.get(7), None);
        assert!(
            images
                .set(9, &[], [0, 1], ImageFormat::Gray, false)
                .is_err()
        );
        assert!(!images.contains(9));
        #[cfg(target_pointer_width = "64")]
        {
            let too_large = u32::MAX as usize + 1;
            assert!(
                images
                    .set(9, &[], [1, too_large], ImageFormat::Gray, false)
                    .unwrap_err()
                    .to_string()
                    .contains("65536")
            );
            assert!(!images.contains(9));
        }
        assert!(
            images
                .update(
                    7,
                    &[1],
                    [0, 0],
                    [1, 1],
                    ImageFormat::Gray,
                    false,
                    None,
                    false
                )
                .is_err()
        );

        images
            .set(
                7,
                &[10, 20, 30, 40, 50, 60],
                [1, 2],
                ImageFormat::Color,
                false,
            )
            .unwrap();
        images
            .set_all(2, [2, 1], ImageColor::GrayAlpha(7, 8), false)
            .unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images.indices(), vec![2, 7]);
        assert!(images.contains(2));
        assert_eq!(images.shape(7), Some([1, 2]));
        // Every key keeps its own format.
        assert_eq!(
            images.get(7),
            Some((vec![10, 20, 30, 40, 50, 60], [1, 2], ImageFormat::Color))
        );
        assert_eq!(images.format(2), Some(ImageFormat::GrayAlpha));
        assert_eq!(images.format(9), None);
        assert_eq!(images.get(2).unwrap().0, [7, 8].repeat(2));

        images
            .update(
                7,
                &[9, 10],
                [0, 1],
                [1, 1],
                ImageFormat::GrayAlpha,
                false,
                None,
                false,
            )
            .unwrap();
        // Gray-alpha and color meet at color-alpha.
        assert_eq!(
            images.get(7),
            Some((
                vec![10, 20, 30, 255, 9, 9, 9, 10],
                [1, 2],
                ImageFormat::ColorAlpha
            ))
        );

        let before = images.get(7).unwrap();
        assert!(
            images
                .update(
                    7,
                    &[],
                    [0, 0],
                    [0, 1],
                    ImageFormat::Gray,
                    false,
                    None,
                    false
                )
                .is_err()
        );
        assert_eq!(images.get(7).unwrap(), before);
        assert!(
            images
                .update(
                    7,
                    &[1],
                    [1, 2],
                    [1, 1],
                    ImageFormat::Gray,
                    false,
                    None,
                    false
                )
                .is_err()
        );
        assert_eq!(images.get(7).unwrap(), before);
        assert!(
            images
                .set(9, &[], [1, 1], ImageFormat::Color, false)
                .is_err()
        );
        assert!(!images.contains(9));

        images
            .set_all(7, [1, 1], ImageColor::Color(1, 2, 3), false)
            .unwrap();
        assert_eq!(images.len(), 2);
        // A fill narrows the image again, to the format of its color.
        assert_eq!(
            images.get(7),
            Some((vec![1, 2, 3], [1, 1], ImageFormat::Color))
        );

        images.remove_index(100).unwrap();
        assert_eq!(images.len(), 2);
        images.remove_index(2).unwrap();
        assert_eq!(images.indices(), vec![7]);
        images.reset().unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn headers_are_stored_with_the_images() {
        type Meta = (u32, String);
        let meta = |n: u32| (n, format!("m{n}"));
        let server = StateServer::new().unwrap();
        let image = Image::<Meta>::new(&server, "root.image", false).unwrap();
        let images = ImageMulti::<Meta>::new(&server, "root.images", false).unwrap();
        let plain = Image::<()>::new(&server, "root.plain", false).unwrap();
        let gray = ImageFormat::Gray;

        assert_eq!(image.header().unwrap(), None);
        assert_eq!(image.get_with_header().unwrap(), None);
        // The set that creates the image needs a header.
        let error = image.set(&[1, 2], [1, 2], gray, false).unwrap_err();
        assert!(error.to_string().contains("needs a header"), "{error}");
        assert!(image.set_header(&meta(0)).is_err());
        image
            .set_with_header(&[1, 2], [1, 2], gray, &meta(1), false)
            .unwrap();
        image.set(&[3, 4], [1, 2], gray, false).unwrap();
        image
            .update(&[5], [0, 0], [1, 1], gray, false, None, false)
            .unwrap();
        assert_eq!(
            image.get_with_header().unwrap(),
            Some((vec![5, 4], [1, 2], gray, meta(1)))
        );
        image
            .update_with_header(&[6], [0, 1], [1, 1], gray, &meta(2), false, None, false)
            .unwrap();
        assert_eq!(image.header().unwrap(), Some(meta(2)));
        image
            .set_all_with_header([1, 1], ImageColor::Gray(7), &meta(3), false)
            .unwrap();
        image.set_header(&meta(4)).unwrap();
        assert_eq!(
            image.get_with_header().unwrap(),
            Some((vec![7], [1, 1], gray, meta(4)))
        );

        assert!(images.set(1, &[1], [1, 1], gray, false).is_err());
        assert!(images.set_header(1, &meta(1)).is_err());
        images
            .set_all_with_header(1, [1, 1], ImageColor::Gray(1), &meta(1), false)
            .unwrap();
        images
            .set_with_header(2, &[2], [1, 1], gray, &meta(2), false)
            .unwrap();
        images.set_header(2, &meta(5)).unwrap();
        assert_eq!(images.header(1).unwrap(), Some(meta(1)));
        assert_eq!(
            images.get_with_header(2).unwrap(),
            Some((vec![2], [1, 1], gray, meta(5)))
        );
        assert_eq!(images.header(3).unwrap(), None);

        // An image without a header type has `()` and takes none.
        plain.set(&[1], [1, 1], gray, false).unwrap();
        assert_eq!(plain.header().unwrap(), Some(()));
        assert!(plain.set_header(&()).is_err());
    }
}
