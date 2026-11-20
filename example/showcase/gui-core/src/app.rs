use std::error::Error;

use eframe::{App, CreationContext};
use egui::{Color32, ColorImage};
use egui_states::{Client, ClientBuilder, ConnectionState, Reconnect, Tls};

mod collections;
mod data;
mod helpers;
mod image;
mod sections;
mod transfers;

use crate::state::{ShowcaseState, sections::PrimaryChoice};

struct ViewState {
    schema: ShowcaseState,
    last_take_text: String,
    empty_take_count: u32,
    number_signal_value: f64,
    enum_signal_value: PrimaryChoice,
    last_take_buffer: Vec<u8>,
    last_take_samples: Vec<f32>,
    last_multi_take_bytes: Vec<(u32, Vec<u8>)>,
    last_multi_take_samples: Vec<(u32, Vec<f32>)>,
    last_multi_take_nested: Vec<(u32, Vec<u16>)>,
    last_offer_note: String,
    /// The summary of `root.data.samples`, computed only when it changed.
    samples_summary: String,
}

impl ViewState {
    fn new(schema: ShowcaseState) -> Self {
        Self {
            schema,
            last_take_text: String::new(),
            empty_take_count: 0,
            number_signal_value: 0.0,
            enum_signal_value: PrimaryChoice::default(),
            last_take_buffer: Vec::new(),
            last_take_samples: Vec::new(),
            last_multi_take_bytes: Vec::new(),
            last_multi_take_samples: Vec::new(),
            last_multi_take_nested: Vec::new(),
            last_offer_note: String::new(),
            samples_summary: "not set".to_string(),
        }
    }
}

pub struct MainApp {
    state: ViewState,
    client: Client,
}

impl MainApp {
    /// Creates the app, connecting to `host` (the default without one) on
    /// `port`, with TLS when `tls` is set.
    pub fn new(
        cc: &CreationContext,
        port: u16,
        host: Option<String>,
        tls: Option<Tls>,
    ) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>> {
        // After the first connect, the client connects again by itself when
        // the server restarts or the connection drops.
        let mut builder = ClientBuilder::<ShowcaseState>::new(cc.egui_ctx.clone(), port)
            .reconnect(Reconnect::default());
        if let Some(host) = host {
            builder = builder.host(host);
        }
        if let Some(tls) = tls {
            builder = builder.tls(tls);
        }
        let (state, client) = builder.build();

        // Shown until the server sends its image; ignored if it already has.
        let placeholder = ColorImage::filled([256, 256], Color32::BLACK);
        state.image.image.initialize(placeholder)?;

        Ok(Box::new(Self {
            state: ViewState::new(state),
            client,
        }))
    }
}

impl eframe::App for MainApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        poll_take_values(&mut self.state);

        egui::Panel::top("top-panel")
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let state = self.client.get_state();
                    let button = match state {
                        ConnectionState::NotConnected => egui::Button::new("Connect"),
                        ConnectionState::Connecting => egui::Button::new("Connecting…"),
                        ConnectionState::WaitingForApproval => {
                            egui::Button::new("Waiting for approval…")
                        }
                        ConnectionState::Connected => {
                            egui::Button::new("Connected").fill(egui::Color32::LIGHT_GREEN)
                        }
                        ConnectionState::Disconnected => {
                            egui::Button::new("Reconnect").fill(egui::Color32::LIGHT_RED)
                        }
                        // A click tries at once instead of after the delay.
                        ConnectionState::Reconnecting => {
                            egui::Button::new("Reconnecting…").fill(egui::Color32::LIGHT_YELLOW)
                        }
                        // `Failed` and any state added in the future.
                        _ => egui::Button::new("Retry").fill(egui::Color32::LIGHT_RED),
                    };
                    // An attempt in progress cannot be started again.
                    let attempting = matches!(
                        state,
                        ConnectionState::Connecting | ConnectionState::WaitingForApproval
                    );
                    let response = ui.add_enabled(!attempting, button);
                    let response = match self.client.last_error() {
                        Some(error) => response.on_hover_text(error),
                        None => response,
                    };
                    if response.clicked() {
                        self.client.connect();
                    }
                    if let Some(reason) = self.client.last_reason() {
                        ui.label(reason.to_string());
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                show_content(ui, &mut self.state);
            });
        });
    }
}

fn poll_take_values(state: &mut ViewState) {
    transfers::poll_offers(state);

    if let Some(value) = state.schema.value_take.take_text.take() {
        state.last_take_text = value;
    }

    if state.schema.value_take.take_empty.take().is_some() {
        state.empty_take_count += 1;
    }

    if let Some(value) = state.schema.data_take.take_buffer.take() {
        state.last_take_buffer = value;
    }

    if let Some(value) = state.schema.data_take.take_samples.take() {
        state.last_take_samples = value;
    }

    for key in 0..4 {
        if let Some(value) = state.schema.data_multi_take.bytes.take(key) {
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
        if let Some(value) = state.schema.data_multi_take.samples.take(key) {
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
        if let Some(value) = state.schema.data_multi_take.nested.buffer.take(key) {
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

fn show_content(ui: &mut egui::Ui, state: &mut ViewState) {
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
    ui.separator();
    transfers::show_transfers(ui, state);
}
