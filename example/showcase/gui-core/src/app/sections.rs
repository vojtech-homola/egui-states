use crate::state::sections::{PrimaryChoice, Summary};

use super::{
    ViewState,
    helpers::{
        format_summary, primary_choice_label, show_optional, show_primary_choice_selector,
        show_secondary_choice_selector,
    },
};

pub(super) fn show_values(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("values", |ui| {
        let values = &state.schema.values;

        ui.label("Value<bool>: root.values.bool_value");
        values
            .bool_value
            .edit_signal(|value| ui.checkbox(value, "bool_value"));

        ui.separator();

        ui.label("Value<i32>: root.values.count");
        values
            .count
            .edit_signal(|count| ui.add(egui::DragValue::new(count).prefix("count ")));

        ui.separator();

        ui.label("ValueAtomic<f64>: root.values.ratio");
        values
            .ratio
            .edit_signal(|ratio| ui.add(egui::Slider::new(ratio, 0.0..=1.0).text("ratio")));

        ui.separator();

        ui.label("Value<f32>, #[state(queue)]: root.values.queued_progress");
        values.queued_progress.edit_signal(|progress| {
            ui.add(egui::Slider::new(progress, 0.0..=1.0).text("queued_progress"))
        });

        ui.separator();

        ui.label("Value<String>: root.values.title");
        values
            .title
            .edit_signal(|title| ui.text_edit_singleline(title));

        ui.separator();

        ui.label("Value<Option<i32>>: root.values.optional_value");
        values.optional_value.edit_signal(|optional_value| {
            show_optional(ui, optional_value, "has value", 0, |ui, value| {
                ui.add(egui::DragValue::new(value).prefix("optional "));
            });
        });

        ui.separator();

        ui.label("Value<[u16; 3]>: root.values.fixed_numbers");
        values.fixed_numbers.edit_signal(|fixed_numbers| {
            ui.horizontal(|ui| {
                for value in fixed_numbers {
                    ui.add(egui::DragValue::new(value));
                }
            });
        });

        ui.separator();

        ui.label("Value<PrimaryChoice>: root.values.primary_choice");
        values
            .primary_choice
            .edit_signal(|choice| show_primary_choice_selector(ui, choice));

        ui.separator();

        ui.label("Nested values: root.values.nested.*");
        values
            .nested
            .secondary_choice
            .edit_signal(|choice| show_secondary_choice_selector(ui, choice));

        values.nested.selected_enum.edit_signal(|selected_enum| {
            show_optional(
                ui,
                selected_enum,
                "selected enum has value",
                PrimaryChoice::default(),
                |ui, value| {
                    show_primary_choice_selector(ui, value);
                },
            );
        });
    });
}

pub(super) fn show_signals(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("signals", |ui| {
        ui.label("Signal<()>, #[state(queue)]: root.signals.empty_signal");
        if ui.button("emit empty signal").clicked() {
            state.schema.signals.empty_signal.set(());
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
            state
                .schema
                .signals
                .number_signal
                .set(state.number_signal_value);
        }
        ui.separator();

        ui.label("Signal<PrimaryChoice>, #[state(queue)]: root.signals.enum_signal");
        let mut enum_changed = false;
        egui::ComboBox::from_label("variant")
            .selected_text(primary_choice_label(state.enum_signal_value))
            .show_ui(ui, |ui| {
                enum_changed |= ui
                    .selectable_value(&mut state.enum_signal_value, PrimaryChoice::A, "A")
                    .changed();
                enum_changed |= ui
                    .selectable_value(&mut state.enum_signal_value, PrimaryChoice::B, "B")
                    .changed();
                enum_changed |= ui
                    .selectable_value(&mut state.enum_signal_value, PrimaryChoice::C, "C")
                    .changed();
            });
        if enum_changed {
            state
                .schema
                .signals
                .enum_signal
                .set(state.enum_signal_value);
        }
    });
}

pub(super) fn show_statics(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("static", |ui| {
        ui.label("Static<String>: root.statics.status_text");
        ui.label(state.schema.statics.status_text.get());

        ui.separator();

        ui.label("Static<Summary>: root.statics.summary");
        let summary = state.schema.statics.summary.get();
        ui.label(format_summary(&summary));

        ui.separator();

        ui.label("StaticAtomic<[f32; 2]>: root.statics.pair");
        let pair = state.schema.statics.pair.get();
        ui.label(format!("[{:.2}, {:.2}]", pair[0], pair[1]));

        ui.separator();

        ui.label("Nested static values: root.statics.nested.*");
        ui.label(state.schema.statics.nested.label.get());
        ui.label(format!(
            "Enum hint: {}",
            primary_choice_label(state.schema.statics.nested.enum_hint.get())
        ));
    });
}

pub(super) fn show_custom_values(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("custom values", |ui| {
        ui.label("Value<Point>: root.custom_values.point");
        state.schema.custom_values.point.edit_signal(|point| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut point.x).speed(0.1).prefix("x "));
                ui.add(egui::DragValue::new(&mut point.y).speed(0.1).prefix("y "));
            });
            ui.text_edit_singleline(&mut point.label);
        });

        ui.separator();

        ui.label("Value<Option<Summary>>: root.custom_values.optional_struct");
        state
            .schema
            .custom_values
            .optional_struct
            .edit_signal(|optional_struct| {
                show_optional(
                    ui,
                    optional_struct,
                    "has value",
                    Summary::default(),
                    |ui, value| {
                        ui.checkbox(&mut value.enabled, "enabled");
                        ui.add(egui::DragValue::new(&mut value.level).prefix("level "));
                        ui.text_edit_singleline(&mut value.name);
                    },
                );
            });
    });
}
