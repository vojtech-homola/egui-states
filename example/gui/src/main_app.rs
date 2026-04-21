use std::error::Error;
use std::fmt::Display;

use eframe::{App, CreationContext};
use egui::{Color32, ColorImage, Rect};
use egui_states::{Client, ClientBuilder, ConnectionState};

use crate::states::{States, TestEnum, TestEnum2, TestStruct2};

pub struct MainApp {
    states: States,
    client: Client,
    last_take_text: String,
    empty_take_count: u32,
    number_signal_value: f64,
    enum_signal_value: TestEnum,
}

impl MainApp {
    pub fn new(
        cc: &CreationContext,
        port: u16,
    ) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>> {
        let builder = ClientBuilder::new().context(cc.egui_ctx.clone());
        let (states, client) = builder.build::<States>(port, 0);

        let image = ColorImage::filled([256, 256], Color32::BLACK);
        states.image.image.initialize(&cc.egui_ctx, image);

        Ok(Box::new(Self {
            states,
            client,
            last_take_text: String::new(),
            empty_take_count: 0,
            number_signal_value: 0.0,
            enum_signal_value: TestEnum::default(),
        }))
    }

    fn poll_take_values(&mut self) {
        if let Some(value) = self.states.value_take.take_text.take() {
            self.last_take_text = value;
        }

        if self.states.value_take.take_empty.take().is_some() {
            self.empty_take_count += 1;
        }
    }

    fn show_values(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("values", |ui| {
            ui.label("Value<bool>: root.values.bool_value");
            let mut bool_value = self.states.values.bool_value.get();
            if ui.checkbox(&mut bool_value, "bool_value").changed() {
                self.states.values.bool_value.set_signal(bool_value);
            }

            ui.separator();

            ui.label("Value<i32>: root.values.count");
            let mut count = self.states.values.count.get();
            if ui
                .add(egui::DragValue::new(&mut count).prefix("count "))
                .changed()
            {
                self.states.values.count.set_signal(count);
            }

            ui.separator();

            ui.label("ValueAtomic<f64>: root.values.ratio");
            let mut ratio = self.states.values.ratio.get();
            if ui
                .add(egui::Slider::new(&mut ratio, 0.0..=1.0).text("ratio"))
                .changed()
            {
                self.states.values.ratio.set_signal(ratio);
            }

            ui.separator();

            ui.label("Value<f32, Queue>: root.values.queued_progress");
            let mut queued_progress = self.states.values.queued_progress.get();
            if ui
                .add(egui::Slider::new(&mut queued_progress, 0.0..=1.0).text("queued_progress"))
                .changed()
            {
                self.states
                    .values
                    .queued_progress
                    .set_signal(queued_progress);
            }

            ui.separator();

            ui.label("Value<String>: root.values.title");
            let mut title = self.states.values.title.get();
            if ui.text_edit_singleline(&mut title).changed() {
                self.states.values.title.set_signal(title);
            }

            ui.separator();

            ui.label("Value<Option<i32>>: root.values.optional_value");
            let mut optional_value = self.states.values.optional_value.get();
            let previous_has_value = optional_value.is_some();
            let mut has_value = previous_has_value;
            let mut optional_changed = ui.checkbox(&mut has_value, "has value").changed();
            if has_value && !previous_has_value {
                optional_value = Some(0);
            }
            if !has_value {
                optional_value = None;
            }
            if let Some(value) = optional_value.as_mut() {
                optional_changed |= ui
                    .add(egui::DragValue::new(value).prefix("optional "))
                    .changed();
            }
            if optional_changed {
                self.states.values.optional_value.set_signal(optional_value);
            }

            ui.separator();

            ui.label("Value<[u16; 3]>: root.values.fixed_numbers");
            let mut fixed_numbers = self.states.values.fixed_numbers.get();
            let mut fixed_changed = false;
            ui.horizontal(|ui| {
                for value in &mut fixed_numbers {
                    fixed_changed |= ui.add(egui::DragValue::new(value)).changed();
                }
            });
            if fixed_changed {
                self.states.values.fixed_numbers.set_signal(fixed_numbers);
            }

            ui.separator();

            ui.label("Value<TestEnum>: root.values.test_enum");
            let mut test_enum = self.states.values.test_enum.get();
            if show_test_enum_selector(ui, &mut test_enum) {
                self.states.values.test_enum.set_signal(test_enum);
            }

            ui.separator();

            ui.label("Nested values: root.values.nested.*");
            let mut secondary_choice = self.states.values.nested.secondary_choice.get();
            if show_test_enum2_selector(ui, &mut secondary_choice) {
                self.states
                    .values
                    .nested
                    .secondary_choice
                    .set_signal(secondary_choice);
            }

            let mut selected_enum = self.states.values.nested.selected_enum.get();
            let previous_has_value = selected_enum.is_some();
            let mut has_value = previous_has_value;
            let mut selected_changed = ui
                .checkbox(&mut has_value, "selected enum has value")
                .changed();
            if has_value && !previous_has_value {
                selected_enum = Some(TestEnum::default());
            }
            if !has_value {
                selected_enum = None;
            }
            if let Some(value) = selected_enum.as_mut() {
                selected_changed |= show_test_enum_selector(ui, value);
            }
            if selected_changed {
                self.states
                    .values
                    .nested
                    .selected_enum
                    .set_signal(selected_enum);
            }
        });
    }

    fn show_signals(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("signals", |ui| {
            ui.label("Signal<(), Queue>: root.signals.empty_signal");
            if ui.button("emit empty signal").clicked() {
                self.states.signals.empty_signal.set(());
            }

            ui.separator();

            ui.label("Signal<f64>: root.signals.number_signal");
            ui.horizontal(|ui| {
                ui.label("value");
                ui.add(
                    egui::DragValue::new(&mut self.number_signal_value)
                        .speed(0.1)
                        .min_decimals(1),
                );
            });
            if ui.button("emit number signal").clicked() {
                self.states
                    .signals
                    .number_signal
                    .set(self.number_signal_value);
            }
            ui.separator();

            ui.label("Signal<TestEnum, Queue>: root.signals.enum_signal");
            let mut enum_changed = false;
            egui::ComboBox::from_label("variant")
                .selected_text(test_enum_label(self.enum_signal_value))
                .show_ui(ui, |ui| {
                    enum_changed |= ui
                        .selectable_value(&mut self.enum_signal_value, TestEnum::A, "A")
                        .changed();
                    enum_changed |= ui
                        .selectable_value(&mut self.enum_signal_value, TestEnum::B, "B")
                        .changed();
                    enum_changed |= ui
                        .selectable_value(&mut self.enum_signal_value, TestEnum::C, "C")
                        .changed();
                });
            if enum_changed {
                self.states
                    .signals
                    .enum_signal
                    .set(self.enum_signal_value);
            }
        });
    }

    fn show_statics(&self, ui: &mut egui::Ui) {
        ui.collapsing("static", |ui| {
            ui.label("Static<String>: root.statics.status_text");
            ui.label(self.states.statics.status_text.get());

            ui.separator();

            ui.label("Static<TestStruct2>: root.statics.summary");
            let summary = self.states.statics.summary.get();
            ui.label(format_test_struct2(&summary));

            ui.separator();

            ui.label("StaticAtomic<[f32; 2]>: root.statics.pair");
            let pair = self.states.statics.pair.get();
            ui.label(format!("[{:.2}, {:.2}]", pair[0], pair[1]));

            ui.separator();

            ui.label("Nested static values: root.statics.nested.*");
            ui.label(self.states.statics.nested.label.get());
            ui.label(format!(
                "Enum hint: {}",
                test_enum_label(self.states.statics.nested.enum_hint.get())
            ));
        });
    }

    fn show_value_take(&self, ui: &mut egui::Ui) {
        ui.collapsing("value_take", |ui| {
            ui.label("ValueTake<String>: root.value_take.take_text");
            ui.label(format!("last received: {}", self.last_take_text));

            ui.separator();

            ui.label("ValueTake<()>: root.value_take.take_empty");
            ui.label(format!("empty take count: {}", self.empty_take_count));
        });
    }

    fn show_custom_values(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("custom values", |ui| {
            ui.label("Value<TestStruct>: root.custom_values.point");
            let mut point = self.states.custom_values.point.get();
            let mut point_changed = false;
            ui.horizontal(|ui| {
                point_changed |= ui
                    .add(egui::DragValue::new(&mut point.x).speed(0.1).prefix("x "))
                    .changed();
                point_changed |= ui
                    .add(egui::DragValue::new(&mut point.y).speed(0.1).prefix("y "))
                    .changed();
            });
            point_changed |= ui.text_edit_singleline(&mut point.label).changed();
            if point_changed {
                self.states.custom_values.point.set_signal(point);
            }

            ui.separator();

            ui.label("Value<Option<TestStruct2>>: root.custom_values.optional_struct");
            let mut optional_struct = self.states.custom_values.optional_struct.get();
            let previous_has_value = optional_struct.is_some();
            let mut has_value = previous_has_value;
            let mut struct_changed = ui.checkbox(&mut has_value, "has value").changed();
            if has_value && !previous_has_value {
                optional_struct = Some(TestStruct2::default());
            }
            if !has_value {
                optional_struct = None;
            }
            if let Some(value) = optional_struct.as_mut() {
                struct_changed |= ui.checkbox(&mut value.enabled, "enabled").changed();
                struct_changed |= ui
                    .add(egui::DragValue::new(&mut value.level).prefix("level "))
                    .changed();
                struct_changed |= ui.text_edit_singleline(&mut value.name).changed();
            }
            if struct_changed {
                self.states
                    .custom_values
                    .optional_struct
                    .set_signal(optional_struct);
            }
        });
    }

    fn show_value_vec(&self, ui: &mut egui::Ui) {
        ui.collapsing("value vec", |ui| {
            ui.label("ValueVec<i32>: root.value_vec.items");
            ui.label("Collection is display-only here. Buttons emit signals and Python mutates the data.");

            self.states.value_vec.items.read(|list| {
                if list.is_empty() {
                    ui.label("empty list");
                } else {
                    for (index, value) in list.iter().enumerate() {
                        ui.label(format!("[{index}] = {value}"));
                    }
                }
            });

            ui.horizontal(|ui| {
                if ui.button("append item").clicked() {
                    self.states.value_vec.actions.append_item.set(());
                }
                if ui.button("remove last").clicked() {
                    self.states.value_vec.actions.remove_last.set(());
                }
                if ui.button("reset demo").clicked() {
                    self.states.value_vec.actions.reset_demo.set(());
                }
            });
        });
    }

    fn show_value_map(&self, ui: &mut egui::Ui) {
        ui.collapsing("value map", |ui| {
            ui.label("ValueMap<u16, u32>: root.value_map.items");
            ui.label("Collection is display-only here. Buttons emit signals and Python mutates the data.");

            let mut items: Vec<_> = self.states.value_map.items.get().into_iter().collect();
            items.sort_by_key(|(key, _)| *key);
            if items.is_empty() {
                ui.label("empty map");
            } else {
                for (key, value) in items {
                    ui.label(format!("{key} => {value}"));
                }
            }

            ui.horizontal(|ui| {
                if ui.button("insert next").clicked() {
                    self.states.value_map.actions.insert_next.set(());
                }
                if ui.button("remove lowest").clicked() {
                    self.states.value_map.actions.remove_lowest.set(());
                }
                if ui.button("reset demo").clicked() {
                    self.states.value_map.actions.reset_demo.set(());
                }
            });
        });
    }

    fn show_data(&self, ui: &mut egui::Ui) {
        ui.collapsing("data", |ui| {
            let (bytes_len, bytes_updated, bytes_preview) = self
                .states
                .data
                .bytes
                .read(|(data, updated)| (data.len(), updated, preview_slice(data)));
            ui.label("Data<u8>: root.data.bytes");
            ui.label(format!(
                "len = {bytes_len}, updated = {bytes_updated}, preview = {bytes_preview}"
            ));

            ui.separator();

            let (samples_len, samples_updated, samples_preview) = self
                .states
                .data
                .samples
                .read(|(data, updated)| (data.len(), updated, preview_f32_slice(data)));
            ui.label("Data<f32>: root.data.samples");
            ui.label(format!(
                "len = {samples_len}, updated = {samples_updated}, preview = {samples_preview}"
            ));

            ui.separator();

            let (buffer_len, buffer_updated, buffer_preview) = self
                .states
                .data
                .nested
                .buffer
                .read(|(data, updated)| (data.len(), updated, preview_slice(data)));
            ui.label("Nested Data<u16>: root.data.nested.buffer");
            ui.label(format!(
                "len = {buffer_len}, updated = {buffer_updated}, preview = {buffer_preview}"
            ));
        });
    }

    fn show_image_section(&self, ui: &mut egui::Ui) {
        ui.collapsing("image", |ui| {
            ui.label("ValueImage: root.image.image");
            show_image(ui, &self.states.image.image.get_id());
            if let Some(size) = self.states.image.image.get_size() {
                ui.label(format!("size = {} x {}", size[1], size[0]));
            }
        });
    }
}

impl eframe::App for MainApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.poll_take_values();

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
                self.show_values(ui);
                ui.separator();
                self.show_signals(ui);
                ui.separator();
                self.show_statics(ui);
                ui.separator();
                self.show_value_take(ui);
                ui.separator();
                self.show_custom_values(ui);
                ui.separator();
                self.show_value_vec(ui);
                ui.separator();
                self.show_value_map(ui);
                ui.separator();
                self.show_data(ui);
                ui.separator();
                self.show_image_section(ui);
            });
        });
    }
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

fn preview_slice<T: Display>(values: &[T]) -> String {
    let preview: Vec<String> = values.iter().take(6).map(ToString::to_string).collect();
    if values.len() > 6 {
        format!("[{}, ...]", preview.join(", "))
    } else {
        format!("[{}]", preview.join(", "))
    }
}

fn preview_f32_slice(values: &[f32]) -> String {
    let preview: Vec<String> = values
        .iter()
        .take(6)
        .map(|value| format!("{value:.2}"))
        .collect();
    if values.len() > 6 {
        format!("[{}, ...]", preview.join(", "))
    } else {
        format!("[{}]", preview.join(", "))
    }
}

fn show_test_enum_selector(ui: &mut egui::Ui, value: &mut TestEnum) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.selectable_value(value, TestEnum::A, "A").changed();
        changed |= ui.selectable_value(value, TestEnum::B, "B").changed();
        changed |= ui.selectable_value(value, TestEnum::C, "C").changed();
    });
    changed
}

fn show_test_enum2_selector(ui: &mut egui::Ui, value: &mut TestEnum2) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.selectable_value(value, TestEnum2::X, "X").changed();
        changed |= ui.selectable_value(value, TestEnum2::Y, "Y").changed();
        changed |= ui.selectable_value(value, TestEnum2::Z, "Z").changed();
    });
    changed
}

fn test_enum_label(value: TestEnum) -> &'static str {
    match value {
        TestEnum::A => "A",
        TestEnum::B => "B",
        TestEnum::C => "C",
    }
}

fn format_test_struct2(value: &TestStruct2) -> String {
    format!(
        "enabled = {}, level = {}, name = {}",
        value.enabled, value.level, value.name
    )
}
