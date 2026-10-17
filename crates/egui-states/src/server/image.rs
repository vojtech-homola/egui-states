//! Server-controlled image storage and image-format helpers.

use std::sync::Arc;

use crate::image_transport::ImageType;
use crate::server_core::image_core::{Image as CoreImage, ImageData};
use crate::server_core::image_core_common::checked_fill_size;
use crate::server_core::image_multi_core::{ImageMulti as CoreImageMulti, ImageMultiData};

use super::state_server::StateServer;
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
    #[inline]
    fn rgba(self) -> [u8; 4] {
        match self {
            Self::Gray(gray) => [gray, gray, gray, 255],
            Self::GrayAlpha(gray, alpha) => [gray, gray, gray, alpha],
            Self::Color(red, green, blue) => [red, green, blue, 255],
            Self::ColorAlpha(red, green, blue, alpha) => [red, green, blue, alpha],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Channel layout of image bytes passed to [`Image::set`] or [`Image::update`].
/// The client expands every format to its four-channel egui texture storage.
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
}

#[derive(Clone)]
/// Server-controlled image retained as RGBA pixels and mirrored to the client.
pub struct Image {
    inner: Arc<CoreImage>,
}

impl Image {
    /// Registers an empty image under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_image(name.into())?;
        Ok(Self { inner })
    }

    /// Returns the image shape as `[height, width]`.
    pub fn shape(&self) -> [usize; 2] {
        self.inner.get_size()
    }

    /// Returns a copy of the RGBA pixels and `[height, width]` shape.
    pub fn get(&self) -> (Vec<u8>, [usize; 2]) {
        self.inner.get_image(|(data, size)| (data.clone(), *size))
    }

    /// Replaces the complete image.
    ///
    /// `size` is `[height, width]`; `data` must contain exactly the number of
    /// bytes implied by `size` and `format`. `update` requests a client repaint.
    ///
    /// # Errors
    ///
    /// Returns an error if the dimensions overflow, the data length does not
    /// match `size` and `format`, or the update cannot be queued.
    pub fn set(
        &self,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        update: bool,
    ) -> Result<()> {
        let stride = self.check_image_data(data, size, format)?;
        let image = ImageData {
            size,
            stride,
            contiguous: true,
            image_type: format.image_type(),
            data: data.as_ptr(),
        };
        self.inner
            .set_image(image, update)
            .map_err(ServerError::new)
    }

    /// Fill an image with one color.
    ///
    /// `shape` is `[height, width]`. When a client is connected, this sends only
    /// a compact header; the server still retains the complete RGBA image for
    /// subsequent reads, updates, and connection synchronization.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or overflowing dimensions, or when the update
    /// cannot be queued.
    pub fn set_all(&self, shape: [usize; 2], color: ImageColor, update: bool) -> Result<()> {
        self.inner
            .set_all_image(shape, color.rgba(), update)
            .map_err(ServerError::new)
    }

    /// Replaces a rectangular image region.
    ///
    /// `origin` and `size` are `[y, x]` and `[height, width]`. With `force`, a
    /// pending update for the same region may be replaced by this one.
    ///
    /// # Errors
    ///
    /// Returns an error if the rectangle lies outside the current image, its
    /// dimensions overflow, its data length is invalid, or it cannot be queued.
    pub fn update(
        &self,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        update: bool,
        force: bool,
    ) -> Result<()> {
        let stride = self.check_image_data(data, size, format)?;
        let image = ImageData {
            size,
            stride,
            contiguous: true,
            image_type: format.image_type(),
            data: data.as_ptr(),
        };
        self.inner
            .update_image(&origin, image, update, force)
            .map_err(ServerError::new)
    }

    fn check_image_data(
        &self,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
    ) -> Result<usize> {
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
        Ok(stride)
    }
}

#[derive(Clone)]
/// Server-controlled images retained as RGBA pixels and indexed by sparse `u32` keys.
pub struct ImageMulti {
    inner: Arc<CoreImageMulti>,
}

impl ImageMulti {
    /// Registers an empty keyed image collection under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_image_multi(name.into())?;
        Ok(Self { inner })
    }

    /// Returns the image shape at `index` as `[height, width]`.
    pub fn shape(&self, index: u32) -> Option<[usize; 2]> {
        self.inner.get_size(index)
    }

    /// Returns a copy of the RGBA pixels and shape at `index`.
    pub fn get(&self, index: u32) -> Option<(Vec<u8>, [usize; 2])> {
        self.inner.get_image(index, |image| {
            image.map(|(data, size)| (data.clone(), *size))
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

    /// Creates or replaces the complete image at `index`.
    pub fn set(
        &self,
        index: u32,
        data: &[u8],
        size: [usize; 2],
        format: ImageFormat,
        update: bool,
    ) -> Result<()> {
        let stride = check_image_multi_data(data, size, format)?;
        self.inner
            .set_image(
                index,
                ImageMultiData {
                    size,
                    stride,
                    contiguous: true,
                    image_type: format.image_type(),
                    data: data.as_ptr(),
                },
                update,
            )
            .map_err(ServerError::new)
    }

    /// Creates or replaces the image at `index` with one uniform color.
    pub fn set_all(
        &self,
        index: u32,
        shape: [usize; 2],
        color: ImageColor,
        update: bool,
    ) -> Result<()> {
        self.inner
            .set_all_image(index, shape, color.rgba(), update)
            .map_err(ServerError::new)
    }

    /// Replaces a rectangular region of the existing image at `index`.
    ///
    /// With `force`, a buffered update may be replaced only when it targets the
    /// same `index` and rectangle. Updates for other indices remain ordered and
    /// are never dropped.
    pub fn update(
        &self,
        index: u32,
        data: &[u8],
        origin: [usize; 2],
        size: [usize; 2],
        format: ImageFormat,
        update: bool,
        force: bool,
    ) -> Result<()> {
        let stride = check_image_multi_data(data, size, format)?;
        self.inner
            .update_image(
                index,
                &origin,
                ImageMultiData {
                    size,
                    stride,
                    contiguous: true,
                    image_type: format.image_type(),
                    data: data.as_ptr(),
                },
                update,
                force,
            )
            .map_err(ServerError::new)
    }

    /// Removes `index`; an absent index is a no-op.
    pub fn remove_index(&self, index: u32, update: bool) -> Result<()> {
        self.inner
            .remove_index(index, update)
            .map_err(ServerError::new)
    }

    /// Removes all image keys.
    pub fn reset(&self, update: bool) -> Result<()> {
        self.inner.reset_images(update).map_err(ServerError::new)
    }
}

fn check_image_multi_data(data: &[u8], size: [usize; 2], format: ImageFormat) -> Result<usize> {
    checked_fill_size(size).map_err(ServerError::new)?;
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
    Ok(stride)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_color_normalizes_to_rgba() {
        assert_eq!(ImageColor::Gray(1).rgba(), [1, 1, 1, 255]);
        assert_eq!(ImageColor::GrayAlpha(1, 2).rgba(), [1, 1, 1, 2]);
        assert_eq!(ImageColor::Color(1, 2, 3).rgba(), [1, 2, 3, 255]);
        assert_eq!(ImageColor::ColorAlpha(1, 2, 3, 4).rgba(), [1, 2, 3, 4]);
    }

    #[test]
    fn image_converts_to_rgba_and_updates() {
        let server = StateServer::new().unwrap();
        let image = Image::new(&server, "root.image").unwrap();

        image
            .set(&[10, 20, 30, 40, 50, 60], [1, 2], ImageFormat::Color, false)
            .unwrap();
        assert_eq!(image.shape(), [1, 2]);
        assert_eq!(image.get().0, vec![10, 20, 30, 255, 40, 50, 60, 255]);

        image
            .update(
                &[7, 8],
                [0, 0],
                [1, 1],
                ImageFormat::GrayAlpha,
                false,
                false,
            )
            .unwrap();
        assert_eq!(image.get().0, vec![7, 7, 7, 8, 40, 50, 60, 255]);
    }

    #[test]
    fn image_rejects_invalid_and_overflowing_dimensions() {
        let server = StateServer::new().unwrap();
        let image = Image::new(&server, "root.image").unwrap();

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
        let image = Image::new(&server, "root.image").unwrap();

        // Nothing has been set yet, so every rectangle is outside the image.
        assert!(
            image
                .update(&[1], [0, 0], [1, 1], ImageFormat::Gray, false, false)
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
                    .update(&data, origin, size, ImageFormat::Gray, false, false)
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
            .update(&[9], [1, 2], [1, 1], ImageFormat::Gray, false, false)
            .unwrap();
        assert_eq!(&image.get().0[20..24], &[9, 9, 9, 255]);
    }

    #[test]
    fn image_set_all_materializes_rgba_data() {
        let server = StateServer::new().unwrap();
        let image = Image::new(&server, "root.image").unwrap();

        image
            .set_all([2, 3], ImageColor::GrayAlpha(7, 8), false)
            .unwrap();

        assert_eq!(image.shape(), [2, 3]);
        assert_eq!(image.get().0, [7, 7, 7, 8].repeat(6));
        assert!(image.set_all([0, 3], ImageColor::Gray(0), false).is_err());
    }

    #[test]
    fn image_multi_sparse_operations_and_validation() {
        let server = StateServer::new().unwrap();
        let images = ImageMulti::new(&server, "root.images").unwrap();

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
                    .contains("protocol limits")
            );
            assert!(!images.contains(9));
        }
        assert!(
            images
                .update(7, &[1], [0, 0], [1, 1], ImageFormat::Gray, false, false,)
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
        assert_eq!(
            images.get(7).unwrap().0,
            vec![10, 20, 30, 255, 40, 50, 60, 255]
        );
        assert_eq!(images.get(2).unwrap().0, [7, 7, 7, 8].repeat(2));

        images
            .update(
                7,
                &[9, 10],
                [0, 1],
                [1, 1],
                ImageFormat::GrayAlpha,
                false,
                false,
            )
            .unwrap();
        assert_eq!(images.get(7).unwrap().0, vec![10, 20, 30, 255, 9, 9, 9, 10]);

        let before = images.get(7).unwrap();
        assert!(
            images
                .update(7, &[], [0, 0], [0, 1], ImageFormat::Gray, false, false,)
                .is_err()
        );
        assert_eq!(images.get(7).unwrap(), before);
        assert!(
            images
                .update(7, &[1], [1, 2], [1, 1], ImageFormat::Gray, false, false,)
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
        assert_eq!(images.get(7).unwrap().0, vec![1, 2, 3, 255]);

        images.remove_index(100, false).unwrap();
        assert_eq!(images.len(), 2);
        images.remove_index(2, false).unwrap();
        assert_eq!(images.indices(), vec![7]);
        images.reset(false).unwrap();
        assert!(images.is_empty());
    }
}
