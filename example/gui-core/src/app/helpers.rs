use std::fmt::Display;

use super::sections::{TestEnum, TestEnum2, TestStruct2};

pub(super) fn preview_slice<T: Display>(values: &[T]) -> String {
    let preview: Vec<String> = values.iter().take(6).map(ToString::to_string).collect();
    if values.len() > 6 {
        format!("[{}, ...]", preview.join(", "))
    } else {
        format!("[{}]", preview.join(", "))
    }
}

pub(super) fn preview_f32_slice(values: &[f32]) -> String {
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

pub(super) fn show_multi_data_preview(ui: &mut egui::Ui, items: &[(u32, usize, String)]) {
    if items.is_empty() {
        ui.label("no indices");
    } else {
        for (index, len, preview) in items {
            ui.label(format!("[{index}] len = {len}, preview = {preview}"));
        }
    }
}

pub(super) fn show_test_enum_selector(ui: &mut egui::Ui, value: &mut TestEnum) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.selectable_value(value, TestEnum::A, "A").changed();
        changed |= ui.selectable_value(value, TestEnum::B, "B").changed();
        changed |= ui.selectable_value(value, TestEnum::C, "C").changed();
    });
    changed
}

pub(super) fn show_test_enum2_selector(ui: &mut egui::Ui, value: &mut TestEnum2) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.selectable_value(value, TestEnum2::X, "X").changed();
        changed |= ui.selectable_value(value, TestEnum2::Y, "Y").changed();
        changed |= ui.selectable_value(value, TestEnum2::Z, "Z").changed();
    });
    changed
}

pub(super) fn test_enum_label(value: TestEnum) -> &'static str {
    match value {
        TestEnum::A => "A",
        TestEnum::B => "B",
        TestEnum::C => "C",
    }
}

pub(super) fn format_test_struct2(value: &TestStruct2) -> String {
    format!(
        "enabled = {}, level = {}, name = {}",
        value.enabled, value.level, value.name
    )
}
