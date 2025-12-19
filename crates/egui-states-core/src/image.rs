use serde::{Deserialize, Serialize};

// use crate::serialization::TYPE_IMAGE;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum ImageType {
    Color,
    ColorAlpha,
    Gray,
    GrayAlpha,
}

impl ImageType {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            ImageType::Color => 3,
            ImageType::ColorAlpha => 4,
            ImageType::Gray => 1,
            ImageType::GrayAlpha => 2,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImageHeader {
    pub image_size: [u32; 2],   // [y, x]
    pub rect: Option<[u32; 4]>, // [y, x, h, w]
    pub image_type: ImageType,
}

impl ImageHeader {
    pub fn serialize<'a>(&self, id: u32, buffer: &'a mut [u8]) -> &'a [u8] {
        // buffer[0] = TYPE_IMAGE;
        buffer[1..5].copy_from_slice(&id.to_le_bytes());

        let len = postcard::to_slice(self, &mut buffer[5..])
            .expect("Failed to serialize image info")
            .len();

        &buffer[0..len + 5]
    }

    pub fn deserialize(data: &[u8]) -> Result<(Self, &[u8]), String> {
        postcard::take_from_bytes(data).map_err(|e| e.to_string())
    }
}
