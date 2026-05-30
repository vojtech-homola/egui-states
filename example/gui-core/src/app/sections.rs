use egui_states::{Queue, Signal, Static, StaticAtomic, Value, ValueAtomic};

use super::{
    State,
    helpers::{
        format_test_struct2, show_test_enum_selector, show_test_enum2_selector, test_enum_label,
    },
};

#[derive(
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    egui_states::Transportable,
)]
pub(super) enum TestEnum {
    #[default]
    A,
    B,
    C,
}

#[derive(
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    egui_states::Transportable,
)]
pub(super) enum TestEnum2 {
    X,
    #[default]
    Y,
    Z,
}

#[derive(
    Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, egui_states::Transportable,
)]
pub(super) struct TestStruct {
    pub x: f32,
    pub y: f32,
    pub label: String,
}

#[derive(
    Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, egui_states::Transportable,
)]
pub(super) struct TestStruct2 {
    pub enabled: bool,
    pub level: u16,
    pub name: String,
}

#[derive(egui_states::State)]
pub(super) struct NestedValueStates {
    pub secondary_choice: Value<TestEnum2>,
    pub selected_enum: Value<Option<TestEnum>>,
}

#[derive(egui_states::State)]
pub(super) struct ValueStates {
    pub bool_value: Value<bool>,
    pub count: Value<i32>,
    pub ratio: ValueAtomic<f64>,
    pub queued_progress: Value<f32, Queue>,
    pub title: Value<String>,
    pub optional_value: Value<Option<i32>>,
    pub fixed_numbers: Value<[u16; 3]>,
    pub test_enum: Value<TestEnum>,
    pub nested: NestedValueStates,
}

#[derive(egui_states::State)]
pub(super) struct StaticStates {
    pub status_text: Static<String>,
    pub summary: Static<TestStruct2>,
    pub pair: StaticAtomic<[f32; 2]>,
    pub nested: NestedStaticStates,
}

#[derive(egui_states::State)]
pub(super) struct NestedStaticStates {
    pub label: Static<String>,
    pub enum_hint: Static<TestEnum>,
}

#[derive(egui_states::State)]
pub(super) struct SignalStates {
    pub empty_signal: Signal<(), Queue>,
    pub number_signal: Signal<f64>,
    pub enum_signal: Signal<TestEnum, Queue>,
}

#[derive(egui_states::State)]
pub(super) struct CustomValueStates {
    pub point: Value<TestStruct>,
    pub optional_struct: Value<Option<TestStruct2>>,
}

pub(super) fn show_values(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("values", |ui| {
        ui.label("Value<bool>: root.values.bool_value");
        let mut bool_value = state.values.bool_value.get();
        if ui.checkbox(&mut bool_value, "bool_value").changed() {
            state.values.bool_value.set_signal(bool_value);
        }

        ui.separator();

        ui.label("Value<i32>: root.values.count");
        let mut count = state.values.count.get();
        if ui
            .add(egui::DragValue::new(&mut count).prefix("count "))
            .changed()
        {
            state.values.count.set_signal(count);
        }

        ui.separator();

        ui.label("ValueAtomic<f64>: root.values.ratio");
        let mut ratio = state.values.ratio.get();
        if ui
            .add(egui::Slider::new(&mut ratio, 0.0..=1.0).text("ratio"))
            .changed()
        {
            state.values.ratio.set_signal(ratio);
        }

        ui.separator();

        ui.label("Value<f32, Queue>: root.values.queued_progress");
        let mut queued_progress = state.values.queued_progress.get();
        if ui
            .add(egui::Slider::new(&mut queued_progress, 0.0..=1.0).text("queued_progress"))
            .changed()
        {
            state.values.queued_progress.set_signal(queued_progress);
        }

        ui.separator();

        ui.label("Value<String>: root.values.title");
        let mut title = state.values.title.get();
        if ui.text_edit_singleline(&mut title).changed() {
            state.values.title.set_signal(title);
        }

        ui.separator();

        ui.label("Value<Option<i32>>: root.values.optional_value");
        let mut optional_value = state.values.optional_value.get();
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
            state.values.optional_value.set_signal(optional_value);
        }

        ui.separator();

        ui.label("Value<[u16; 3]>: root.values.fixed_numbers");
        let mut fixed_numbers = state.values.fixed_numbers.get();
        let mut fixed_changed = false;
        ui.horizontal(|ui| {
            for value in &mut fixed_numbers {
                fixed_changed |= ui.add(egui::DragValue::new(value)).changed();
            }
        });
        if fixed_changed {
            state.values.fixed_numbers.set_signal(fixed_numbers);
        }

        ui.separator();

        ui.label("Value<TestEnum>: root.values.test_enum");
        let mut test_enum = state.values.test_enum.get();
        if show_test_enum_selector(ui, &mut test_enum) {
            state.values.test_enum.set_signal(test_enum);
        }

        ui.separator();

        ui.label("Nested values: root.values.nested.*");
        let mut secondary_choice = state.values.nested.secondary_choice.get();
        if show_test_enum2_selector(ui, &mut secondary_choice) {
            state
                .values
                .nested
                .secondary_choice
                .set_signal(secondary_choice);
        }

        let mut selected_enum = state.values.nested.selected_enum.get();
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
            state.values.nested.selected_enum.set_signal(selected_enum);
        }
    });
}

pub(super) fn show_signals(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("signals", |ui| {
        ui.label("Signal<(), Queue>: root.signals.empty_signal");
        if ui.button("emit empty signal").clicked() {
            state.signals.empty_signal.set(());
        }

        ui.separator();

        ui.label("Signal<f64>: root.signals.number_signal");
        ui.horizontal(|ui| {
            ui.label("value");
            ui.add(
                egui::DragValue::new(&mut state.number_signal_value)
                    .speed(0.1)
                    .min_decimals(1),
            );
        });
        if ui.button("emit number signal").clicked() {
            state.signals.number_signal.set(state.number_signal_value);
        }
        ui.separator();

        ui.label("Signal<TestEnum, Queue>: root.signals.enum_signal");
        let mut enum_changed = false;
        egui::ComboBox::from_label("variant")
            .selected_text(test_enum_label(state.enum_signal_value))
            .show_ui(ui, |ui| {
                enum_changed |= ui
                    .selectable_value(&mut state.enum_signal_value, TestEnum::A, "A")
                    .changed();
                enum_changed |= ui
                    .selectable_value(&mut state.enum_signal_value, TestEnum::B, "B")
                    .changed();
                enum_changed |= ui
                    .selectable_value(&mut state.enum_signal_value, TestEnum::C, "C")
                    .changed();
            });
        if enum_changed {
            state.signals.enum_signal.set(state.enum_signal_value);
        }
    });
}

pub(super) fn show_statics(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("static", |ui| {
        ui.label("Static<String>: root.statics.status_text");
        ui.label(state.statics.status_text.get());

        ui.separator();

        ui.label("Static<TestStruct2>: root.statics.summary");
        let summary = state.statics.summary.get();
        ui.label(format_test_struct2(&summary));

        ui.separator();

        ui.label("StaticAtomic<[f32; 2]>: root.statics.pair");
        let pair = state.statics.pair.get();
        ui.label(format!("[{:.2}, {:.2}]", pair[0], pair[1]));

        ui.separator();

        ui.label("Nested static values: root.statics.nested.*");
        ui.label(state.statics.nested.label.get());
        ui.label(format!(
            "Enum hint: {}",
            test_enum_label(state.statics.nested.enum_hint.get())
        ));
    });
}

pub(super) fn show_custom_values(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("custom values", |ui| {
        ui.label("Value<TestStruct>: root.custom_values.point");
        let mut point = state.custom_values.point.get();
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
            state.custom_values.point.set_signal(point);
        }

        ui.separator();

        ui.label("Value<Option<TestStruct2>>: root.custom_values.optional_struct");
        let mut optional_struct = state.custom_values.optional_struct.get();
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
            state
                .custom_values
                .optional_struct
                .set_signal(optional_struct);
        }
    });
}
