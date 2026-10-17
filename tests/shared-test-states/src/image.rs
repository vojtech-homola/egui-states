use egui_states::{Image, ImageMulti};

#[derive(egui_states::State)]
pub struct ImageStates {
    pub image: Image,
    pub images: ImageMulti,
}
