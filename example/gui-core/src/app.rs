use std::error::Error;

use eframe::{App, CreationContext};
use egui::{Color32, ColorImage};
use egui_states::{Client, ClientBuilder, ConnectionState, State as EguiState, StatesCreator};

mod collections;
mod data;
mod helpers;
mod image;
mod sections;

use collections::{ValueMapStates, ValueVecStates};
use data::{DataStates, DataTakeStates, MultiDataStates, MultiDataTakeStates, ValueTakeStates};
use image::ImageStates;
use sections::{CustomValueStates, SignalStates, StaticStates, TestEnum, ValueStates};

pub struct State {
    values: ValueStates,
    signals: SignalStates,
    statics: StaticStates,
    value_take: ValueTakeStates,
    custom_values: CustomValueStates,
    value_vec: ValueVecStates,
    value_map: ValueMapStates,
    data: DataStates,
    data_take: DataTakeStates,
    multi_data: MultiDataStates,
    data_multi_take: MultiDataTakeStates,
    image: ImageStates,
    last_take_text: String,
    empty_take_count: u32,
    number_signal_value: f64,
    enum_signal_value: TestEnum,
    last_take_buffer: Vec<u8>,
    last_take_samples: Vec<f32>,
    last_multi_take_bytes: Vec<(u32, Vec<u8>)>,
    last_multi_take_samples: Vec<(u32, Vec<f32>)>,
    last_multi_take_nested: Vec<(u32, Vec<u16>)>,
}

impl EguiState for State {
    const NAME: &'static str = "State";

    fn new(c: &mut impl StatesCreator) -> Self {
        Self {
            values: c.substate("values"),
            signals: c.substate("signals"),
            statics: c.substate("statics"),
            value_take: c.substate("value_take"),
            custom_values: c.substate("custom_values"),
            value_vec: c.substate("value_vec"),
            value_map: c.substate("value_map"),
            data: c.substate("data"),
            data_take: c.substate("data_take"),
            multi_data: c.substate("multi_data"),
            data_multi_take: c.substate("data_multi_take"),
            image: c.substate("image"),
            last_take_text: String::new(),
            empty_take_count: 0,
            number_signal_value: 0.0,
            enum_signal_value: TestEnum::default(),
            last_take_buffer: Vec::new(),
            last_take_samples: Vec::new(),
            last_multi_take_bytes: Vec::new(),
            last_multi_take_samples: Vec::new(),
            last_multi_take_nested: Vec::new(),
        }
    }
}

pub struct MainApp {
    state: State,
    client: Client,
}

impl MainApp {
    pub fn new(
        cc: &CreationContext,
        port: u16,
    ) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>> {
        let builder = ClientBuilder::new().context(cc.egui_ctx.clone());
        let (state, client) = builder.build::<State>(port, 0);

        let image = ColorImage::filled([256, 256], Color32::BLACK);
        state.image.image.initialize(&cc.egui_ctx, image);

        Ok(Box::new(Self { state, client }))
    }
}

impl eframe::App for MainApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        poll_take_values(&mut self.state);

        egui::Panel::top("top-panel")
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
            egui::ScrollArea::vertical().show(ui, |ui| {
                show_content(ui, &mut self.state);
            });
        });
    }
}

fn poll_take_values(state: &mut State) {
    if let Some(value) = state.value_take.take_text.take() {
        state.last_take_text = value;
    }

    if state.value_take.take_empty.take().is_some() {
        state.empty_take_count += 1;
    }

    if let Some(value) = state.data_take.take_buffer.take() {
        state.last_take_buffer = value;
    }

    if let Some(value) = state.data_take.take_samples.take() {
        state.last_take_samples = value;
    }

    for key in 0..4 {
        if let Some(value) = state.data_multi_take.bytes.take(key) {
            if let Some(entry) = state
                .last_multi_take_bytes
                .iter_mut()
                .find(|(k, _)| *k == key)
            {
                entry.1 = value;
            } else {
                state.last_multi_take_bytes.push((key, value));
            }
        }
        if let Some(value) = state.data_multi_take.samples.take(key) {
            if let Some(entry) = state
                .last_multi_take_samples
                .iter_mut()
                .find(|(k, _)| *k == key)
            {
                entry.1 = value;
            } else {
                state.last_multi_take_samples.push((key, value));
            }
        }
        if let Some(value) = state.data_multi_take.nested.buffer.take(key) {
            if let Some(entry) = state
                .last_multi_take_nested
                .iter_mut()
                .find(|(k, _)| *k == key)
            {
                entry.1 = value;
            } else {
                state.last_multi_take_nested.push((key, value));
            }
        }
    }
}

fn show_content(ui: &mut egui::Ui, state: &mut State) {
    sections::show_values(ui, state);
    ui.separator();
    sections::show_signals(ui, state);
    ui.separator();
    sections::show_statics(ui, state);
    ui.separator();
    data::show_value_take(ui, state);
    ui.separator();
    data::show_data_take(ui, state);
    ui.separator();
    sections::show_custom_values(ui, state);
    ui.separator();
    collections::show_value_vec(ui, state);
    ui.separator();
    collections::show_value_map(ui, state);
    ui.separator();
    data::show_data(ui, state);
    ui.separator();
    data::show_multi_data(ui, state);
    ui.separator();
    data::show_multi_data_take(ui, state);
    ui.separator();
    image::show_image_section(ui, state);
}
