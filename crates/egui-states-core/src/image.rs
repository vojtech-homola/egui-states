use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum ImageType {
    Color,
    ColorAlpha,
    Gray,
    GrayAlpha,
}

#[derive(Serialize, Deserialize)]
pub struct ImageInfo {
    pub image_size: [usize; 2],   // [y, x]
    pub rect: Option<[usize; 4]>, // [y, x, h, w]
    pub image_type: ImageType,
}
