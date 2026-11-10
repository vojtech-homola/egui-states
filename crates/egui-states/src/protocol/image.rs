use serde::{Deserialize, Serialize};

#[cfg(any(feature = "server", feature = "python"))]
use crate::protocol::{FastVec, ServerHeader, serialize, serialize_heap};

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

/// Converts `source` pixels in `image_type` to RGBA, four bytes per pixel,
/// into `target`, which holds exactly that many bytes.
///
/// Bytes are copied as they are: gray becomes `(g, g, g, 255)`, gray-alpha
/// `(g, g, g, a)` and color gets an opaque alpha. `target` may be
/// uninitialized; every byte of it is written.
pub(crate) fn to_rgba(
    image_type: ImageType,
    source: &[u8],
    target: &mut [std::mem::MaybeUninit<u8>],
) {
    let pixels = source.len() / image_type.bytes_per_pixel();
    assert_eq!(target.len(), pixels * 4, "RGBA target size");
    let out = target.as_mut_ptr().cast::<[u8; 4]>();
    // SAFETY (all arms): `target` holds `pixels` four-byte slots that do not
    // overlap `source`; unaligned writes need no alignment.
    match image_type {
        ImageType::ColorAlpha => unsafe {
            std::ptr::copy_nonoverlapping(source.as_ptr(), out.cast::<u8>(), source.len())
        },
        ImageType::Color => {
            for (i, pixel) in source.as_chunks::<3>().0.iter().enumerate() {
                let quad = [pixel[0], pixel[1], pixel[2], 255];
                unsafe { out.add(i).write_unaligned(quad) };
            }
        }
        ImageType::Gray => {
            for (i, &gray) in source.iter().enumerate() {
                unsafe { out.add(i).write_unaligned([gray, gray, gray, 255]) };
            }
        }
        ImageType::GrayAlpha => {
            for (i, pixel) in source.as_chunks::<2>().0.iter().enumerate() {
                let quad = [pixel[0], pixel[0], pixel[0], pixel[1]];
                unsafe { out.add(i).write_unaligned(quad) };
            }
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

#[cfg(feature = "client")]
impl ImageHeader {
    /// Whether the message asks for a repaint; the first and middle chunks of
    /// an image set never do.
    pub(crate) fn update_flag(&self) -> bool {
        match self {
            Self::Set(ImageSetHeader::All(_, update) | ImageSetHeader::End(_, update), _) => {
                *update
            }
            Self::Set(ImageSetHeader::Start(..) | ImageSetHeader::Batch(_), _) => false,
            Self::Update(_, _, update) | Self::Fill(_, _, update) => *update,
        }
    }

    /// Whether the client acknowledges this message, also when it fails.
    pub(crate) fn requires_ack(&self) -> bool {
        matches!(
            self,
            Self::Set(ImageSetHeader::All(..) | ImageSetHeader::End(..), _)
                | Self::Update(..)
                | Self::Fill(..)
        )
    }
}

#[cfg(feature = "client")]
impl ImageMultiHeader {
    pub(crate) fn update_flag(&self) -> bool {
        match self {
            Self::Remove(_, update) | Self::Reset(update) => *update,
            Self::Modify(_, header, _) => header.update_flag(),
        }
    }

    pub(crate) fn requires_ack(&self) -> bool {
        matches!(self, Self::Modify(_, header, _) if header.requires_ack())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the byte semantics every peer relies on.
    #[test]
    fn pixels_convert_to_rgba_bytes_as_they_are() {
        let convert = |image_type, source: &[u8]| {
            let mut target = vec![
                std::mem::MaybeUninit::uninit();
                source.len() / ImageType::bytes_per_pixel(&image_type) * 4
            ];
            to_rgba(image_type, source, &mut target);
            target
                .into_iter()
                .map(|byte| unsafe { byte.assume_init() })
                .collect::<Vec<u8>>()
        };
        assert_eq!(
            convert(ImageType::Gray, &[7, 9]),
            [7, 7, 7, 255, 9, 9, 9, 255]
        );
        assert_eq!(convert(ImageType::GrayAlpha, &[7, 3]), [7, 7, 7, 3]);
        assert_eq!(convert(ImageType::Color, &[1, 2, 3]), [1, 2, 3, 255]);
        assert_eq!(convert(ImageType::ColorAlpha, &[1, 2, 3, 4]), [1, 2, 3, 4]);
    }

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
