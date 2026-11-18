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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

    #[cfg(any(feature = "server", feature = "python"))]
    fn has_color(self) -> bool {
        matches!(self, Self::Color | Self::ColorAlpha)
    }

    #[cfg(any(feature = "server", feature = "python"))]
    fn has_alpha(self) -> bool {
        matches!(self, Self::GrayAlpha | Self::ColorAlpha)
    }

    /// The smallest format that holds every pixel of both: color if either
    /// has color, alpha if either has alpha.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn union(self, other: Self) -> Self {
        match (
            self.has_color() || other.has_color(),
            self.has_alpha() || other.has_alpha(),
        ) {
            (false, false) => Self::Gray,
            (false, true) => Self::GrayAlpha,
            (true, false) => Self::Color,
            (true, true) => Self::ColorAlpha,
        }
    }
}

/// Converts `source` pixels in `from` to the format `to`, which must hold
/// `from` (see [`ImageType::union`]), into `target`, which holds exactly that
/// many bytes.
///
/// Bytes are copied as they are, as [`to_rgba`] does: gray becomes equal
/// color channels and a missing alpha becomes 255. `target` may be
/// uninitialized; every byte of it is written. Bytes of `source` after its
/// last whole pixel are ignored.
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) fn widen(
    from: ImageType,
    to: ImageType,
    source: &[u8],
    target: &mut [std::mem::MaybeUninit<u8>],
) {
    assert_eq!(from.union(to), to, "{to:?} does not hold {from:?}");
    let pixels = source.len() / from.bytes_per_pixel();
    assert_eq!(target.len(), pixels * to.bytes_per_pixel(), "target size");
    // Only whole pixels: the copies below write as many bytes as they read.
    let source = &source[..pixels * from.bytes_per_pixel()];
    let out = target.as_mut_ptr().cast::<u8>();
    // SAFETY (all arms): `target` holds `pixels` pixels of `to` that do not
    // overlap `source`; byte writes need no alignment.
    match (from, to) {
        _ if from == to => unsafe {
            std::ptr::copy_nonoverlapping(source.as_ptr(), out, source.len())
        },
        (_, ImageType::ColorAlpha) => to_rgba(from, source, target),
        (ImageType::Gray, ImageType::GrayAlpha) => {
            for (i, &gray) in source.iter().enumerate() {
                unsafe { out.cast::<[u8; 2]>().add(i).write_unaligned([gray, 255]) };
            }
        }
        (ImageType::Gray, ImageType::Color) => {
            for (i, &gray) in source.iter().enumerate() {
                unsafe { out.cast::<[u8; 3]>().add(i).write_unaligned([gray; 3]) };
            }
        }
        _ => unreachable!("{to:?} does not hold {from:?}"),
    }
}

/// [`widen`] into a target that is already initialized.
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) fn widen_into(from: ImageType, to: ImageType, source: &[u8], target: &mut [u8]) {
    // SAFETY: `MaybeUninit<u8>` has the layout of `u8`, and `widen` writes
    // only initialized bytes.
    let target = unsafe { &mut *(target as *mut [u8] as *mut [std::mem::MaybeUninit<u8>]) };
    widen(from, to, source, target);
}

/// Converts `source` pixels in `image_type` to RGBA, four bytes per pixel,
/// into `target`, which holds exactly that many bytes.
///
/// Bytes are copied as they are: gray becomes `(g, g, g, 255)`, gray-alpha
/// `(g, g, g, a)` and color gets an opaque alpha. `target` may be
/// uninitialized; every byte of it is written. Bytes of `source` after its
/// last whole pixel are ignored.
pub(crate) fn to_rgba(
    image_type: ImageType,
    source: &[u8],
    target: &mut [std::mem::MaybeUninit<u8>],
) {
    let pixels = source.len() / image_type.bytes_per_pixel();
    assert_eq!(target.len(), pixels * 4, "RGBA target size");
    // Only whole pixels: the RGBA copy below writes as many bytes as it reads.
    let source = &source[..pixels * image_type.bytes_per_pixel()];
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

/// Chunk framing of a rectangle update; it mirrors [`ImageSetHeader`]. The
/// client applies the rectangle once, at `All` or `End`.
#[derive(Serialize, Deserialize)]
pub(crate) enum ImageUpdateHeader {
    All([u32; 4], bool),  // [x, y, w, h], update
    Start([u32; 4], u32), // [x, y, w, h], pixels
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
    Set(ImageSetHeader, ImageType),       // header
    Update(ImageUpdateHeader, ImageType), // header
    Fill([u32; 2], [u8; 4], bool),        // [width, height], RGBA, update
}

#[cfg(feature = "client")]
impl ImageHeader {
    /// Whether the message asks for a repaint; the first and middle chunks of
    /// an image set or update never do.
    pub(crate) fn update_flag(&self) -> bool {
        match self {
            Self::Set(ImageSetHeader::All(_, update) | ImageSetHeader::End(_, update), _)
            | Self::Update(
                ImageUpdateHeader::All(_, update) | ImageUpdateHeader::End(_, update),
                _,
            )
            | Self::Fill(_, _, update) => *update,
            Self::Set(ImageSetHeader::Start(..) | ImageSetHeader::Batch(_), _)
            | Self::Update(ImageUpdateHeader::Start(..) | ImageUpdateHeader::Batch(_), _) => false,
        }
    }

    /// Whether the client acknowledges this message, also when it fails.
    pub(crate) fn requires_ack(&self) -> bool {
        matches!(
            self,
            Self::Set(ImageSetHeader::All(..) | ImageSetHeader::End(..), _)
                | Self::Update(ImageUpdateHeader::All(..) | ImageUpdateHeader::End(..), _)
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
        // The bytes of an incomplete last pixel are not copied past the target.
        assert_eq!(
            convert(ImageType::ColorAlpha, &[1, 2, 3, 4, 5]),
            [1, 2, 3, 4]
        );
        assert_eq!(convert(ImageType::Color, &[1, 2, 3, 4]), [1, 2, 3, 255]);
    }

    #[cfg(any(feature = "server", feature = "python"))]
    #[test]
    fn pixels_widen_to_every_format_that_holds_them() {
        use ImageType::*;

        let widened = |from, to, source: &[u8]| {
            let pixels = source.len() / ImageType::bytes_per_pixel(&from);
            let mut target = vec![0; pixels * ImageType::bytes_per_pixel(&to)];
            widen_into(from, to, source, &mut target);
            target
        };
        let gray: &[u8] = &[7, 9];
        assert_eq!(widened(Gray, Gray, gray), [7, 9]);
        assert_eq!(widened(Gray, GrayAlpha, gray), [7, 255, 9, 255]);
        assert_eq!(widened(Gray, Color, gray), [7, 7, 7, 9, 9, 9]);
        assert_eq!(
            widened(Gray, ColorAlpha, gray),
            [7, 7, 7, 255, 9, 9, 9, 255]
        );
        assert_eq!(widened(GrayAlpha, GrayAlpha, &[7, 3]), [7, 3]);
        assert_eq!(widened(GrayAlpha, ColorAlpha, &[7, 3]), [7, 7, 7, 3]);
        assert_eq!(widened(Color, Color, &[1, 2, 3]), [1, 2, 3]);
        assert_eq!(widened(Color, ColorAlpha, &[1, 2, 3]), [1, 2, 3, 255]);
        assert_eq!(widened(ColorAlpha, ColorAlpha, &[1, 2, 3, 4]), [1, 2, 3, 4]);
        // The bytes of an incomplete last pixel are not copied past the target.
        assert_eq!(
            widened(ColorAlpha, ColorAlpha, &[1, 2, 3, 4, 5]),
            [1, 2, 3, 4]
        );
        assert_eq!(widened(GrayAlpha, GrayAlpha, &[7, 3, 9]), [7, 3]);
        assert_eq!(widened(Color, Color, &[1, 2, 3, 4]), [1, 2, 3]);
    }

    #[cfg(any(feature = "server", feature = "python"))]
    #[test]
    fn the_union_of_two_formats_holds_both() {
        use ImageType::*;

        let all = [Gray, GrayAlpha, Color, ColorAlpha];
        for a in all {
            assert_eq!(a.union(a), a);
            assert_eq!(a.union(Gray), a);
            assert_eq!(a.union(ColorAlpha), ColorAlpha);
        }
        assert_eq!(GrayAlpha.union(Color), ColorAlpha);
        assert_eq!(Color.union(GrayAlpha), ColorAlpha);
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
