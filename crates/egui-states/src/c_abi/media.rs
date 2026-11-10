//! Validation of image and numeric buffers passed to the server.

use crate::protocol::image::ImageType;
use crate::server_core::states::data::DataSlice;
use crate::server_core::states::image::common::checked_image_size;
use crate::server_core::states::image::single::ImageData;

use super::error::{Error, Result};
use super::kinds::DataKind;

/// Channel layout of image bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ImageFormat {
    /// One byte per pixel: grayscale intensity.
    Gray = 0,
    /// Two bytes per pixel: grayscale intensity and alpha.
    GrayAlpha = 1,
    /// Three bytes per pixel: red, green, and blue.
    Rgb = 2,
    /// Four bytes per pixel: red, green, blue, and alpha.
    Rgba = 3,
}

impl ImageFormat {
    /// Converts a C value, or returns `None` if it is not a valid format.
    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Gray),
            1 => Some(Self::GrayAlpha),
            2 => Some(Self::Rgb),
            3 => Some(Self::Rgba),
            _ => None,
        }
    }

    fn image_type(self) -> ImageType {
        match self {
            Self::Gray => ImageType::Gray,
            Self::GrayAlpha => ImageType::GrayAlpha,
            Self::Rgb => ImageType::Color,
            Self::Rgba => ImageType::ColorAlpha,
        }
    }
}

/// Image pixels passed to the server, with rows `row_stride` bytes apart.
///
/// A `row_stride` of zero means tightly packed rows. Both dimensions must be
/// between 1 and 65,536 pixels.
#[derive(Clone, Copy, Debug)]
pub struct ImageInput<'a> {
    /// Pixel bytes; at least `(height - 1) * stride + width * bytes per pixel`.
    pub data: &'a [u8],
    /// Number of rows.
    pub height: usize,
    /// Number of pixels per row.
    pub width: usize,
    /// Bytes between the starts of consecutive rows, or `0` for packed rows.
    pub row_stride: usize,
    /// Channel layout.
    pub format: ImageFormat,
}

impl<'a> ImageInput<'a> {
    /// Checks the input and views it as an image of the server core.
    pub(crate) fn image_data(&self) -> Result<ImageData<'a>> {
        let size = [self.height, self.width];
        checked_image_size(size).map_err(Error::out_of_range)?;
        let image_type = self.format.image_type();
        let overflow = || Error::out_of_range("image dimensions overflow");
        let packed = self
            .width
            .checked_mul(image_type.bytes_per_pixel())
            .ok_or_else(overflow)?;
        let stride = match self.row_stride {
            0 => packed,
            stride if stride < packed => {
                return Err(Error::invalid_argument(format!(
                    "row stride {stride} is smaller than a packed row of {packed} bytes"
                )));
            }
            stride => stride,
        };
        let required = (self.height - 1)
            .checked_mul(stride)
            .and_then(|rows| rows.checked_add(packed))
            .ok_or_else(overflow)?;
        if self.data.len() < required {
            return Err(Error::invalid_argument(format!(
                "image data holds {} bytes, but its shape needs {required}",
                self.data.len()
            )));
        }
        Ok(ImageData {
            size,
            stride,
            image_type,
            data: self.data,
        })
    }
}

/// Wraps native-endian numbers of `kind` for the data cores, which copy them.
pub(crate) fn data_slice(kind: DataKind, bytes: &[u8]) -> Result<DataSlice<'_>> {
    if !bytes.len().is_multiple_of(kind.size()) {
        return Err(Error::invalid_argument(format!(
            "{} bytes are not a whole number of {} values",
            bytes.len(),
            kind.name()
        )));
    }
    Ok(DataSlice {
        bytes,
        data_type: kind.data_type(),
        header: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_abi::ErrorCode;

    fn input(data: &[u8], height: usize, width: usize, row_stride: usize) -> ImageInput<'_> {
        ImageInput {
            data,
            height,
            width,
            row_stride,
            format: ImageFormat::Rgb,
        }
    }

    #[test]
    fn images_are_checked_against_their_shape() {
        let data = [0u8; 64];
        let packed = input(&data[..12], 2, 2, 0).image_data().unwrap();
        assert!(packed.contiguous());
        assert_eq!(packed.stride, 6);
        assert_eq!(packed.size, [2, 2]);

        // The last row needs no padding after it.
        let strided = input(&data[..16], 2, 2, 10).image_data().unwrap();
        assert!(!strided.contiguous());
        assert_eq!(strided.stride, 10);

        for (image, code, message) in [
            (
                input(&data[..15], 2, 2, 10),
                ErrorCode::InvalidArgument,
                "needs 16",
            ),
            (
                input(&data, 2, 2, 5),
                ErrorCode::InvalidArgument,
                "row stride 5",
            ),
            (
                input(&data, 0, 2, 0),
                ErrorCode::OutOfRange,
                "cannot be zero",
            ),
            (input(&data, 1, 65537, 0), ErrorCode::OutOfRange, "65536"),
        ] {
            let error = image.image_data().err().unwrap();
            assert_eq!(error.code, code);
            assert!(error.message.contains(message), "{}", error.message);
        }
    }

    #[test]
    fn data_must_be_whole_elements() {
        let slice = data_slice(DataKind::U32, &[0; 8]).unwrap();
        assert_eq!(slice.bytes.len(), 8);
        assert_eq!(slice.data_type, DataKind::U32.data_type());
        assert!(data_slice(DataKind::U32, &[0; 7]).is_err());
        assert!(data_slice(DataKind::F64, &[]).unwrap().bytes.is_empty());
    }
}
