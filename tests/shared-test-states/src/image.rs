use egui_states::{Image, ImageMulti};

#[derive(egui_states::State)]
pub struct ImageStates {
    pub image: Image,
    /// Uses the low-priority lane.
    #[state(low_priority)]
    pub images: ImageMulti,
}
