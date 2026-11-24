use egui_states::{Image, ImageMulti};

use crate::data::HeaderMeta;

#[derive(egui_states::State)]
pub struct ImageStates {
    pub image: Image,
    /// Uses the low-priority lane.
    #[state(low_priority)]
    pub images: ImageMulti,
    /// An image and images with a typed header, changed by command 37.
    pub framed: Image<HeaderMeta>,
    pub framed_keys: ImageMulti<HeaderMeta>,
}
