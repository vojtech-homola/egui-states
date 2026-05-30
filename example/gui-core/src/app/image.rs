use egui::{Color32, Rect};
use egui_states::Image;

use super::State;

#[derive(egui_states::State)]
pub(super) struct ImageStates {
    pub image: Image,
}

pub(super) fn show_image_section(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("image", |ui| {
        ui.label("Image: root.image.image");
        show_image(ui, &state.image.image.get_id());
        if let Some(size) = state.image.image.get_size() {
            ui.label(format!("size = {} x {}", size[1], size[0]));
        }
    });
}

fn show_image(ui: &mut egui::Ui, texture_id: &Option<egui::TextureId>) {
    const UV: Rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let (response, painter) = ui.allocate_painter([192.0, 192.0].into(), egui::Sense::HOVER);

    match texture_id {
        Some(texture_id) => {
            painter.image(*texture_id, response.rect, UV, Color32::WHITE);
        }
        None => {
            painter.rect_filled(response.rect, 0.0, Color32::DARK_GRAY);
        }
    }
}
