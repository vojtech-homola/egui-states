use egui_states::{Data, DataMulti, DataMultiTake, DataTake, ValueTake};

use super::{
    State,
    helpers::{preview_f32_slice, preview_slice, show_multi_data_preview},
};

pub(super) struct ValueTakeStates {
    pub take_text: ValueTake<String>,
    pub take_empty: ValueTake<()>,
}

pub(super) struct NestedDataStates {
    pub buffer: Data<u16>,
}

pub(super) struct DataStates {
    pub bytes: Data<u8>,
    pub samples: Data<f32>,
    pub nested: NestedDataStates,
}

pub(super) struct NestedMultiDataStates {
    pub buffer: DataMulti<u16>,
}

pub(super) struct MultiDataStates {
    pub bytes: DataMulti<u8>,
    pub samples: DataMulti<f32>,
    pub nested: NestedMultiDataStates,
}

pub(super) struct DataTakeStates {
    pub take_buffer: DataTake<u8>,
    pub take_samples: DataTake<f32>,
}

pub(super) struct NestedMultiDataTakeStates {
    pub buffer: DataMultiTake<u16>,
}

pub(super) struct MultiDataTakeStates {
    pub bytes: DataMultiTake<u8>,
    pub samples: DataMultiTake<f32>,
    pub nested: NestedMultiDataTakeStates,
}

impl egui_states::State for ValueTakeStates {
    const NAME: &'static str = "ValueTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            take_text: c.value_take("take_text"),
            take_empty: c.value_take("take_empty"),
        }
    }
}

impl egui_states::State for DataTakeStates {
    const NAME: &'static str = "DataTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            take_buffer: c.data_take("take_buffer"),
            take_samples: c.data_take("take_samples"),
        }
    }
}

impl egui_states::State for NestedMultiDataTakeStates {
    const NAME: &'static str = "NestedMultiDataTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            buffer: c.data_multi_take("buffer"),
        }
    }
}

impl egui_states::State for MultiDataTakeStates {
    const NAME: &'static str = "MultiDataTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            bytes: c.data_multi_take("bytes"),
            samples: c.data_multi_take("samples"),
            nested: c.substate("nested"),
        }
    }
}

impl egui_states::State for NestedDataStates {
    const NAME: &'static str = "NestedDataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            buffer: c.data("buffer"),
        }
    }
}

impl egui_states::State for DataStates {
    const NAME: &'static str = "DataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            bytes: c.data("bytes"),
            samples: c.data("samples"),
            nested: c.substate("nested"),
        }
    }
}

impl egui_states::State for NestedMultiDataStates {
    const NAME: &'static str = "NestedMultiDataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            buffer: c.data_multi("buffer"),
        }
    }
}

impl egui_states::State for MultiDataStates {
    const NAME: &'static str = "MultiDataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            bytes: c.data_multi("bytes"),
            samples: c.data_multi("samples"),
            nested: c.substate("nested"),
        }
    }
}

pub(super) fn show_value_take(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("value_take", |ui| {
        ui.label("ValueTake<String>: root.value_take.take_text");
        ui.label(format!("last received: {}", state.last_take_text));

        ui.separator();

        ui.label("ValueTake<()>: root.value_take.take_empty");
        ui.label(format!("empty take count: {}", state.empty_take_count));
    });
}

pub(super) fn show_data_take(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("data_take", |ui| {
        ui.label("DataTake<u8>: root.data_take.take_buffer");
        let buffer_preview = preview_slice(&state.last_take_buffer);
        ui.label(format!(
            "last received: len = {}, preview = {}",
            state.last_take_buffer.len(),
            buffer_preview
        ));

        ui.separator();

        ui.label("DataTake<f32>: root.data_take.take_samples");
        let samples_preview = preview_f32_slice(&state.last_take_samples);
        ui.label(format!(
            "last received: len = {}, preview = {}",
            state.last_take_samples.len(),
            samples_preview
        ));
    });
}

pub(super) fn show_data(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("data", |ui| {
        let (bytes_len, bytes_preview) = state
            .data
            .bytes
            .read(|data| (data.len(), preview_slice(data)));
        ui.label("Data<u8>: root.data.bytes");
        ui.label(format!("len = {bytes_len}, preview = {bytes_preview}"));

        ui.separator();

        let (samples_len, samples_preview) = state
            .data
            .samples
            .read(|data| (data.len(), preview_f32_slice(data)));
        ui.label("Data<f32>: root.data.samples");
        ui.label(format!("len = {samples_len}, preview = {samples_preview}"));

        ui.separator();

        let (buffer_len, buffer_preview) = state
            .data
            .nested
            .buffer
            .read(|data| (data.len(), preview_slice(data)));
        ui.label("Nested Data<u16>: root.data.nested.buffer");
        ui.label(format!("len = {buffer_len}, preview = {buffer_preview}"));
    });
}

pub(super) fn show_multi_data(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("multi data", |ui| {
        let mut bytes_items = state.multi_data.bytes.read_all(|data| {
            data.iter()
                .map(|(index, values)| (*index, values.len(), preview_slice(values)))
                .collect::<Vec<_>>()
        });
        bytes_items.sort_by_key(|(index, _, _)| *index);
        ui.label("DataMulti<u8>: root.multi_data.bytes");
        show_multi_data_preview(ui, &bytes_items);

        ui.separator();

        let mut samples_items = state.multi_data.samples.read_all(|data| {
            data.iter()
                .map(|(index, values)| (*index, values.len(), preview_f32_slice(values)))
                .collect::<Vec<_>>()
        });
        samples_items.sort_by_key(|(index, _, _)| *index);
        ui.label("DataMulti<f32>: root.multi_data.samples");
        show_multi_data_preview(ui, &samples_items);

        ui.separator();

        let mut buffer_items = state.multi_data.nested.buffer.read_all(|data| {
            data.iter()
                .map(|(index, values)| (*index, values.len(), preview_slice(values)))
                .collect::<Vec<_>>()
        });
        buffer_items.sort_by_key(|(index, _, _)| *index);
        ui.label("Nested DataMulti<u16>: root.multi_data.nested.buffer");
        show_multi_data_preview(ui, &buffer_items);
    });
}

pub(super) fn show_multi_data_take(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("multi_data_take", |ui| {
        ui.label("DataMultiTake<u8>: root.data_multi_take.bytes");
        let mut bytes_items = state.last_multi_take_bytes.clone();
        bytes_items.sort_by_key(|(index, _)| *index);
        for (index, values) in bytes_items {
            let preview = preview_slice(&values);
            ui.label(format!(
                "  key {}: len = {}, preview = {}",
                index,
                values.len(),
                preview
            ));
        }
        if state.last_multi_take_bytes.is_empty() {
            ui.label("  (no takes yet)");
        }

        ui.separator();

        ui.label("DataMultiTake<f32>: root.data_multi_take.samples");
        let mut samples_items = state.last_multi_take_samples.clone();
        samples_items.sort_by_key(|(index, _)| *index);
        for (index, values) in samples_items {
            let preview = preview_f32_slice(&values);
            ui.label(format!(
                "  key {}: len = {}, preview = {}",
                index,
                values.len(),
                preview
            ));
        }
        if state.last_multi_take_samples.is_empty() {
            ui.label("  (no takes yet)");
        }

        ui.separator();

        ui.label("Nested DataMultiTake<u16>: root.data_multi_take.nested.buffer");
        let mut buffer_items = state.last_multi_take_nested.clone();
        buffer_items.sort_by_key(|(index, _)| *index);
        for (index, values) in buffer_items {
            let preview = preview_slice(&values);
            ui.label(format!(
                "  key {}: len = {}, preview = {}",
                index,
                values.len(),
                preview
            ));
        }
        if state.last_multi_take_nested.is_empty() {
            ui.label("  (no takes yet)");
        }
    });
}
