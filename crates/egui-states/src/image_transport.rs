use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use crate::serialization::{FastVec, ServerHeader, serialize_heap};

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

#[cfg(feature = "server")]
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
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ImageHeader {
    Set(ImageSetHeader, ImageType),    // header
    Update([u32; 4], ImageType, bool), // [x, y, w, h], image_type, update
}
