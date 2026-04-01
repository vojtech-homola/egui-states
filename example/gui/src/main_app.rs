use std::error::Error;

use eframe::{App, CreationContext};
use egui::{Color32, ColorImage, Rect};
use egui_states::{Client, ClientBuilder, ConnectionState};

use crate::states::{States, TestEnum};

pub struct MainApp {
    states: States,
    client: Client,
    take_1: String,
    take_2: bool,
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

        Ok(Box::new(Self { states, client, take_1: String::new(), take_2: false }))
    }
}

impl eframe::App for MainApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::Panel::top("top-panlel")
            .show_separator_line(false)
            .show_inside(ui, |ui| {
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

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // image --------------------------------------------------
            let texture_id = self.states.image.get_id();
            const UV: Rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let (response, painter) =
                ui.allocate_painter([256.0, 256.0].into(), egui::Sense::HOVER);
            match texture_id {
                Some(texture_id) => {
                    painter.image(texture_id, response.rect, UV, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(response.rect, 0.0, Color32::DARK_GRAY);
                }
            }

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
            // let mut value2 = Diff::new(&self.states.value2);
            // let mut step = 0.01;
            // let box_ = WheelBoxF::new(&mut value2.v, 3)
            //     .desired_width(100.0)
            //     .single_step(&mut step);
            // ui.add(box_);
            // value2.set_signal();

            let s = self.states.my_sub_state.stat.get();
            ui.label(format!("Static atomic value: [{}, {}]", s[0], s[1]));

            let text = match self.states.test_enum.get() {
                TestEnum::A => "A",
                TestEnum::B => "B",
                TestEnum::C => "C",
            };
            ui.label(format!("Test enum: {}", text));

            if ui.button("Set test enum to C").clicked() {
                self.states.test_enum.set_signal(TestEnum::C);
            }

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

            // take --------------------------------------------------
            if let Some(val) = self.states.value_take_1.take() {
                ui.label(format!("Take 1 value: {}", val));
                self.take_1 = val;
            } else {
                ui.label(format!("Take 1 value: {}", self.take_1));
            }

            if let Some(_) = self.states.value_take_2.take() {
                ui.label(format!("Take 2 value: {}", self.take_2));
                self.take_2 = !self.take_2;
            } else {
                ui.label(format!("Take 2 value: {}", self.take_2));
            }
        });

        // std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
