use egui_states::{Image, ImageMulti};

/// Describes the showcase image; the server sends it as the header of the
/// image, so the GUI shows each image with its own description.
#[egui_states::typed]
#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub frame: u32,
    /// Micrometers per pixel.
    pub scale: f32,
}

#[derive(egui_states::State)]
pub struct ImageStates {
    pub image: Image<ImageInfo>,
    pub images: ImageMulti,
}
