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
}

impl MainApp {
    pub fn new(
        cc: &CreationContext,
        port: u16,
    ) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>> {
        let builder = ClientBuilder::new().context(cc.egui_ctx.clone());
        let (states, client) = builder.build::<States>(port, 0);

        let image = ColorImage::filled([256, 256], Color32::BLACK);
        states.data.image.initialize(&cc.egui_ctx, image);

        Ok(Box::new(Self {
            states,
            client,
            last_take_text: String::new(),
            empty_take_count: 0,
        }))
    }

    fn poll_take_values(&mut self) {
        if let Some(value) = self.states.events.take_text.take() {
            self.last_take_text = value;
        }

        if self.states.events.take_empty.take().is_some() {
            self.empty_take_count += 1;
        }
    }

    fn show_scalars(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("scalars", |ui| {
            ui.label("Value<bool>: root.scalars.bool_value");
            let mut bool_value = self.states.scalars.bool_value.get();
            if ui.checkbox(&mut bool_value, "bool_value").changed() {
                self.states.scalars.bool_value.set_signal(bool_value);
            }

            ui.separator();

            ui.label("Value<i32>: root.scalars.count");
            let mut count = self.states.scalars.count.get();
            if ui.add(egui::DragValue::new(&mut count).prefix("count ")).changed() {
                self.states.scalars.count.set_signal(count);
            }

            ui.separator();

            ui.label("ValueAtomic<f64>: root.scalars.ratio");
            let mut ratio = self.states.scalars.ratio.get();
            if ui
                .add(egui::Slider::new(&mut ratio, 0.0..=1.0).text("ratio"))
                .changed()
            {
                self.states.scalars.ratio.set_signal(ratio);
            }

            ui.separator();

            ui.label("Value<f32, Queue>: root.scalars.queued_progress");
            let mut queued_progress = self.states.scalars.queued_progress.get();
            if ui
                .add(egui::Slider::new(&mut queued_progress, 0.0..=1.0).text("queued_progress"))
                .changed()
            {
                self.states.scalars.queued_progress.set_signal(queued_progress);
            }

            ui.separator();

            ui.label("Value<String>: root.scalars.title");
            let mut title = self.states.scalars.title.get();
            if ui.text_edit_singleline(&mut title).changed() {
                self.states.scalars.title.set_signal(title);
            }

            ui.separator();

            ui.label("Value<Option<i32>>: root.scalars.optional_value");
            let mut optional_value = self.states.scalars.optional_value.get();
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
                self.states.scalars.optional_value.set_signal(optional_value);
            }

            ui.separator();

            ui.label("Value<[u16; 3]>: root.scalars.fixed_numbers");
            let mut fixed_numbers = self.states.scalars.fixed_numbers.get();
            let mut fixed_changed = false;
            ui.horizontal(|ui| {
                for value in &mut fixed_numbers {
                    fixed_changed |= ui.add(egui::DragValue::new(value)).changed();
                }
            });
            if fixed_changed {
                self.states.scalars.fixed_numbers.set_signal(fixed_numbers);
            }

            ui.separator();

            ui.label("Value<TestEnum>: root.scalars.test_enum");
            let mut test_enum = self.states.scalars.test_enum.get();
            if show_test_enum_selector(ui, &mut test_enum) {
                self.states.scalars.test_enum.set_signal(test_enum);
            }
        });
    }

    fn show_statics(&self, ui: &mut egui::Ui) {
        ui.collapsing("statics", |ui| {
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
        });
    }

    fn show_custom(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("custom", |ui| {
            ui.label("Value<TestStruct>: root.custom.point");
            let mut point = self.states.custom.point.get();
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
                self.states.custom.point.set_signal(point);
            }

            ui.separator();

            ui.label("Value<TestEnum2>: root.custom.choice");
            let mut choice = self.states.custom.choice.get();
            if show_test_enum2_selector(ui, &mut choice) {
                self.states.custom.choice.set_signal(choice);
            }

            ui.separator();

            ui.label("Value<Option<TestStruct2>>: root.custom.optional_struct");
            let mut optional_struct = self.states.custom.optional_struct.get();
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
                self.states.custom.optional_struct.set_signal(optional_struct);
            }
        });
    }

    fn show_collections(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("collections", |ui| {
            ui.label("Value<Vec<u32>>: root.collections.plain_vec_value");
            let mut plain_vec = self.states.collections.plain_vec_value.get();
            let mut plain_vec_changed = false;
            for value in &mut plain_vec {
                plain_vec_changed |= ui.add(egui::DragValue::new(value)).changed();
            }
            ui.horizontal(|ui| {
                if ui.button("append item").clicked() {
                    let next = plain_vec.last().copied().unwrap_or(0) + 1;
                    plain_vec.push(next);
                    plain_vec_changed = true;
                }
                if ui.button("remove last").clicked() && plain_vec.pop().is_some() {
                    plain_vec_changed = true;
                }
            });
            if plain_vec_changed {
                self.states.collections.plain_vec_value.set_signal(plain_vec);
            }

            ui.separator();

            ui.label("ValueVec<i32>: root.collections.list");
            self.states.collections.list.read(|list| {
                if list.is_empty() {
                    ui.label("empty list");
                } else {
                    for (index, value) in list.iter().enumerate() {
                        ui.label(format!("[{index}] = {value}"));
                    }
                }
            });

            ui.separator();

            ui.label("ValueMap<u16, u32>: root.collections.map");
            let mut items: Vec<_> = self.states.collections.map.get().into_iter().collect();
            items.sort_by_key(|(key, _)| *key);
            if items.is_empty() {
                ui.label("empty map");
            } else {
                for (key, value) in items {
                    ui.label(format!("{key} => {value}"));
                }
            }
        });
    }

    fn show_events(&self, ui: &mut egui::Ui) {
        ui.collapsing("events", |ui| {
            ui.label("Signal<(), Queue>: root.events.empty_signal");
            if ui.button("emit empty signal").clicked() {
                self.states.events.empty_signal.set(());
            }

            ui.separator();

            ui.label("Signal<f64>: root.events.number_signal");
            if ui.button("emit current ratio").clicked() {
                self.states.events.number_signal.set(self.states.scalars.ratio.get());
            }

            ui.separator();

            ui.label("Signal<TestEnum, Queue>: root.events.enum_signal");
            if ui.button("emit current enum").clicked() {
                self.states.events.enum_signal.set(self.states.scalars.test_enum.get());
            }

            ui.separator();

            ui.label("ValueTake<String>: root.events.take_text");
            ui.label(format!("last received: {}", self.last_take_text));

            ui.separator();

            ui.label("ValueTake<()>: root.events.take_empty");
            ui.label(format!("empty take count: {}", self.empty_take_count));
        });
    }

    fn show_data(&self, ui: &mut egui::Ui) {
        ui.collapsing("data", |ui| {
            ui.label("ValueImage: root.data.image");
            show_image(ui, &self.states.data.image.get_id());

            ui.separator();

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
        });
    }

    fn show_nested(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("nested", |ui| {
            ui.label("Static<String>: root.nested.label");
            ui.label(self.states.nested.label.get());

            ui.separator();

            ui.label("Value<i32, Queue>: root.nested.counter");
            let mut counter = self.states.nested.counter.get();
            if ui
                .add(egui::DragValue::new(&mut counter).prefix("counter "))
                .changed()
            {
                self.states.nested.counter.set_signal(counter);
            }

            ui.separator();

            ui.label("Value<Option<TestEnum>>: root.nested.inner.selected");
            let mut selected = self.states.nested.inner.selected.get();
            let previous_has_value = selected.is_some();
            let mut has_value = previous_has_value;
            let mut selected_changed = ui.checkbox(&mut has_value, "has value").changed();
            if has_value && !previous_has_value {
                selected = Some(TestEnum::default());
            }
            if !has_value {
                selected = None;
            }
            if let Some(value) = selected.as_mut() {
                selected_changed |= show_test_enum_selector(ui, value);
            }
            if selected_changed {
                self.states.nested.inner.selected.set_signal(selected);
            }

            ui.separator();

            ui.label("StaticAtomic<[f32; 2]>: root.nested.inner.pair");
            let pair = self.states.nested.inner.pair.get();
            ui.label(format!("[{:.2}, {:.2}]", pair[0], pair[1]));

            ui.separator();

            ui.label("Value<bool>: root.nested.inner.leaf.enabled");
            let mut enabled = self.states.nested.inner.leaf.enabled.get();
            if ui.checkbox(&mut enabled, "enabled").changed() {
                self.states.nested.inner.leaf.enabled.set_signal(enabled);
            }

            ui.separator();

            ui.label("Value<String>: root.nested.inner.leaf.message");
            let mut message = self.states.nested.inner.leaf.message.get();
            if ui.text_edit_singleline(&mut message).changed() {
                self.states.nested.inner.leaf.message.set_signal(message);
            }

            ui.separator();

            let (buffer_len, buffer_updated, buffer_preview) = self
                .states
                .nested
                .inner
                .leaf
                .buffer
                .read(|(data, updated)| (data.len(), updated, preview_slice(data)));
            ui.label("Data<u16>: root.nested.inner.leaf.buffer");
            ui.label(format!(
                "len = {buffer_len}, updated = {buffer_updated}, preview = {buffer_preview}"
            ));
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

                    ui.label("Example tree covers all example-facing state wrappers except Graphs.");
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.show_scalars(ui);
                ui.separator();
                self.show_statics(ui);
                ui.separator();
                self.show_custom(ui);
                ui.separator();
                self.show_collections(ui);
                ui.separator();
                self.show_events(ui);
                ui.separator();
                self.show_data(ui);
                ui.separator();
                self.show_nested(ui);
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
    let preview: Vec<String> = values.iter().take(6).map(|value| format!("{value:.2}")).collect();
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

fn format_test_struct2(value: &TestStruct2) -> String {
    format!(
        "enabled = {}, level = {}, name = {}",
        value.enabled, value.level, value.name
    )
}
