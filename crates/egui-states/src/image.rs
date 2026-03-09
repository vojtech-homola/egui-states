use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum ImageType {
    Color,
    ColorAlpha,
    Gray,
    GrayAlpha,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ImageHeader {
    pub image_size: [u32; 2],   // [y, x]
    pub rect: Option<[u32; 4]>, // [y, x, h, w]
    pub image_type: ImageType,
}
