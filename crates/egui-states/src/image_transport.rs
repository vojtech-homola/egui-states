use serde::{Deserialize, Serialize};

#[cfg(any(feature = "server", feature = "python"))]
use crate::serialization::{FastVec, ServerHeader, serialize, serialize_heap};

pub(crate) const MAX_IMAGE_SIDE: usize = 1 << 16;

/// Validate either `[height, width]` or `[width, height]` without allocating.
/// Empty-image policy belongs to the caller.
pub(crate) fn checked_image_shape(shape: [usize; 2]) -> Result<[u32; 2], String> {
    if shape.iter().any(|&side| side > MAX_IMAGE_SIDE) {
        return Err(format!(
            "Image dimensions exceed maximum side length of {MAX_IMAGE_SIDE}"
        ));
    }
    // The shared limit fits in u32, including its inclusive upper boundary.
    Ok([shape[0] as u32, shape[1] as u32])
}

pub(crate) fn checked_image_bytes(
    shape: [usize; 2],
    bytes_per_pixel: usize,
) -> Result<usize, String> {
    checked_image_shape(shape)?;
    shape[0]
        .checked_mul(shape[1])
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| "Image dimensions overflow".to_string())
}

/// Validate a rectangle in either axis order, including its exclusive end.
pub(crate) fn checked_image_bounds(origin: [usize; 2], shape: [usize; 2]) -> Result<(), String> {
    checked_image_shape(shape)?;
    let end = [
        origin[0]
            .checked_add(shape[0])
            .ok_or_else(|| "Image coordinates overflow".to_string())?,
        origin[1]
            .checked_add(shape[1])
            .ok_or_else(|| "Image coordinates overflow".to_string())?,
    ];
    checked_image_shape(end)?;
    Ok(())
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum ImageType {
    Color,
    ColorAlpha,
    Gray,
    GrayAlpha,
}

impl ImageType {
    #[inline]
    pub(crate) fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Color => 3,
            Self::ColorAlpha => 4,
            Self::Gray => 1,
            Self::GrayAlpha => 2,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ImageSetHeader {
    All([u32; 2], bool),  // [x, y], update
    Start([u32; 2], u32), // [x, y], pixels
    Batch(u32),           // pixels
    End(u32, bool),       // pixels, update
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ImageMultiHeader {
    Remove(u32, bool),             // index, update
    Modify(u32, ImageHeader, u32), // index, image header, payload size
    Reset(bool),                   // update
}

#[cfg(any(feature = "server", feature = "python"))]
pub(crate) fn serialize_image_multi_header(
    id: u64,
    index: u32,
    header: ImageHeader,
    size: u32,
) -> Result<FastVec<32>, ()> {
    let header = ServerHeader::ImageMulti(id, ImageMultiHeader::Modify(index, header, size));
    serialize_heap(&header)
}

#[cfg(any(feature = "server", feature = "python"))]
impl ImageSetHeader {
    pub(crate) fn serialize(
        self,
        id: u64,
        image_type: ImageType,
        size: u32,
    ) -> Result<FastVec<32>, ()> {
        let header = &ServerHeader::Image(id, ImageHeader::Set(self, image_type), size);
        serialize_heap(header)
    }

    pub(crate) fn serialize_multi(
        self,
        id: u64,
        index: u32,
        image_type: ImageType,
        size: u32,
    ) -> Result<FastVec<32>, ()> {
        serialize_image_multi_header(id, index, ImageHeader::Set(self, image_type), size)
    }
}

#[cfg(any(feature = "server", feature = "python"))]
impl ImageMultiHeader {
    pub(crate) fn serialize(self, id: u64) -> Result<FastVec<32>, ()> {
        serialize(&ServerHeader::ImageMulti(id, self))
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ImageHeader {
    Set(ImageSetHeader, ImageType),    // header
    Update([u32; 4], ImageType, bool), // [x, y, w, h], image_type, update
    Fill([u32; 2], [u8; 4], bool),     // [width, height], RGBA, update
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_shape_limit_is_inclusive_and_independent_of_allocation() {
        for shape in [
            [MAX_IMAGE_SIDE, 1],
            [1, MAX_IMAGE_SIDE],
            [MAX_IMAGE_SIDE; 2],
            [0, 0],
        ] {
            assert!(checked_image_shape(shape).is_ok());
        }
        for shape in [
            [MAX_IMAGE_SIDE + 1, 1],
            [1, MAX_IMAGE_SIDE + 1],
            [usize::MAX, 0],
        ] {
            assert!(checked_image_shape(shape).unwrap_err().contains("65536"));
        }
        #[cfg(target_pointer_width = "32")]
        assert!(checked_image_bytes([MAX_IMAGE_SIDE; 2], 4).is_err());
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            checked_image_bytes([MAX_IMAGE_SIDE; 2], 4).unwrap(),
            1 << 34
        );
    }

    #[test]
    fn image_rectangles_check_exclusive_ends_and_overflow() {
        for axis in 0..2 {
            let mut origin = [0, 0];
            origin[axis] = MAX_IMAGE_SIDE - 1;
            checked_image_bounds(origin, [1, 1]).unwrap();
            origin[axis] += 1;
            assert!(
                checked_image_bounds(origin, [1, 1])
                    .unwrap_err()
                    .contains("65536")
            );
            origin[axis] = usize::MAX;
            assert!(checked_image_bounds(origin, [1, 1]).is_err());
        }
    }
}
