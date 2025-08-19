use std::error::Error;

use eframe::{App, CreationContext};
use egui::{Color32, ColorImage, Rect};
use egui_pysync::{ClientBuilder, ConnectionState, UIState};

use crate::states::States;

pub struct MainApp {
    states: States,
    ui_state: UIState,
}

impl MainApp {
    pub fn new(cc: &CreationContext) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>> {
        let mut builder = ClientBuilder::new();
        let states = States::new(builder.creator());
        let host = std::net::Ipv4Addr::new(127, 0, 0, 1);
        let ui_state = builder.build(cc.egui_ctx.clone(), host, 8081, 0);

        let image = ColorImage::filled([1024, 1024], Color32::BLACK);
        states.image.initialize(&cc.egui_ctx, image);

        Ok(Box::new(Self { states, ui_state }))
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top-panlel")
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let button = match self.ui_state.get_state() {
                        ConnectionState::NotConnected => egui::Button::new("Connect"),
                        ConnectionState::Connected => {
                            egui::Button::new("Connected").fill(egui::Color32::LIGHT_GREEN)
                        }
                        ConnectionState::Disconnected => {
                            egui::Button::new("Reconnect").fill(egui::Color32::LIGHT_RED)
                        }
                    };
                    if ui.add(button).clicked() {
                        self.ui_state.connect();
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let texture_id = self.states.image.get_id();
            const UV: Rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let (response, painter) =
                ui.allocate_painter([1024.0, 1024.0].into(), egui::Sense::HOVER);
            painter.image(
                texture_id,
                response.rect.translate(egui::vec2(15.0, 50.0)) / 1.5,
                UV,
                Color32::WHITE,
            );
        });
    }
}
