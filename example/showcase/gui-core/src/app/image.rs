use egui::{Color32, Rect};

use super::ViewState;

pub(super) fn show_image_section(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("image", |ui| {
        ui.label("Image: root.image.image");
        show_image(ui, &state.schema.image.image.get_id());
        if let Some(size) = state.schema.image.image.get_size() {
            ui.label(format!("size = {} x {}", size[1], size[0]));
        }
        // The header arrives with the pixels it describes.
        ui.label(state.schema.image.image.read_header(|info| match info {
            Some(info) => format!("frame {}, {} µm/px", info.frame, info.scale),
            None => "no header yet".to_string(),
        }));

        ui.separator();
        ui.label(format!(
            "ImageMulti: root.image.images ({} images)",
            state.schema.image.images.len()
        ));
        for index in state.schema.image.images.indices() {
            let size = state.schema.image.images.get_size(index);
            ui.label(match size {
                Some(size) => format!("index {index}: {} x {}", size[1], size[0]),
                None => format!("index {index}"),
            });
            show_image(ui, &state.schema.image.images.get_id(index));
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
