use std::sync::Arc;

use crate::image_transport::ImageType;
use crate::server_core::image_core::{Image as CoreImage, ImageData};

use super::state_server::StateServer;
use super::{Result, ServerError};

/// A uniform image color for [`Image::set_all`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageColor {
    Gray(u8),
    GrayAlpha(u8, u8),
    Color(u8, u8, u8),
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
pub enum ImageFormat {
    Color,
    ColorAlpha,
    Gray,
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
pub struct Image {
    inner: Arc<CoreImage>,
}

impl Image {
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (_, inner) = server.add_image(name.into())?;
        Ok(Self { inner })
    }

    pub fn shape(&self) -> [usize; 2] {
        self.inner.get_size()
    }

    pub fn get(&self) -> (Vec<u8>, [usize; 2]) {
        self.inner.get_image(|(data, size)| (data.clone(), *size))
    }

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
    pub fn set_all(&self, shape: [usize; 2], color: ImageColor, update: bool) -> Result<()> {
        self.inner
            .set_all_image(shape, color.rgba(), update)
            .map_err(ServerError::new)
    }

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
        let server = StateServer::new(0).unwrap();
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
        let server = StateServer::new(0).unwrap();
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

    #[test]
    fn image_set_all_materializes_rgba_data() {
        let server = StateServer::new(0).unwrap();
        let image = Image::new(&server, "root.image").unwrap();

        image
            .set_all([2, 3], ImageColor::GrayAlpha(7, 8), false)
            .unwrap();

        assert_eq!(image.shape(), [2, 3]);
        assert_eq!(image.get().0, [7, 7, 7, 8].repeat(6));
        assert!(image.set_all([0, 3], ImageColor::Gray(0), false).is_err());
    }
}
