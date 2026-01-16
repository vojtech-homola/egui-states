use std::error::Error;

use eframe::{App, CreationContext};
use egui::{Color32, ColorImage, Rect};
use egui_states::{Client, ClientBuilder, ConnectionState, Diff};

use egui_states_widgets::WheelBoxF;

use crate::states::States;

pub struct MainApp {
    states: States,
    client: Client,
}

impl MainApp {
    pub fn new(
        cc: &CreationContext,
        port: u16,
    ) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>> {
        let builder = ClientBuilder::new().context(cc.egui_ctx.clone());
        let (states, client) = builder.build::<States>(port, 0);

        let image = ColorImage::filled([1024, 1024], Color32::BLACK);
        states.image.initialize(&cc.egui_ctx, image);

        Ok(Box::new(Self { states, client }))
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top-panlel")
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let button = match self.client.get_state() {
                        ConnectionState::NotConnected => egui::Button::new("Connect"),
                        ConnectionState::Connected => {
                            egui::Button::new("Connected").fill(egui::Color32::LIGHT_GREEN)
                        }
                        ConnectionState::Disconnected => {
                            egui::Button::new("Reconnect").fill(egui::Color32::LIGHT_RED)
                        }
                    };
                    if ui.add(button).clicked() {
                        self.client.connect();
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // image --------------------------------------------------
            let texture_id = self.states.image.get_id();
            const UV: Rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let (response, painter) =
                ui.allocate_painter([512.0, 512.0].into(), egui::Sense::HOVER);
            painter.image(texture_id, response.rect, UV, Color32::WHITE);

            let g = self.states.graphs.get(0);
            if let Some(g) = g {
                ui.label(format!("Graph points: {}", g.y.len()));
            } else {
                ui.label("No graph");
            }

            // value --------------------------------------------------
            let mut value = self.states.value.get();
            if ui
                .add(egui::Slider::new(&mut value, 0.0..=1.0).text("value"))
                .changed()
            {
                self.states.value.set_signal(value);
            }

            // value2 --------------------------------------------------
            let mut value2 = Diff::new(&self.states.value2);
            let mut step = 0.01;
            let box_ = WheelBoxF::new(&mut value2.v, 3)
                .desired_width(100.0)
                .single_step(&mut step);
            ui.add(box_);
            value2.set_signal();

            //map --------------------------------------------------
            self.states.collections.map.read(|m| {
                for (k, v) in m {
                    ui.label(format!("Map key: {}, value: {}", k, v));
                }
            });

            ui.separator();

            //list --------------------------------------------------
            self.states.collections.list.read(|l| {
                for v in l {
                    ui.label(format!("List item: {}", v));
                }
            });

            ui.separator();

            let l = self.states.map.get();
            for v in l {
                ui.label(format!("List item: {}", v));
            }

            // signal --------------------------------------------------
            if ui.button("Emit empty signal").clicked() {
                self.states.empty_signal.set(());
            }
        });

        // std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
