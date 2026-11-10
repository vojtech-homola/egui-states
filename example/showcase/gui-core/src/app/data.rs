use super::{
    ViewState,
    helpers::{preview_f32_slice, preview_slice, show_multi_data_preview},
};

pub(super) fn show_value_take(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("value_take", |ui| {
        ui.label("ValueTake<String>: root.value_take.take_text");
        ui.label(format!("last received: {}", state.last_take_text));

        ui.separator();

        ui.label("ValueTake<()>: root.value_take.take_empty");
        ui.label(format!("empty take count: {}", state.empty_take_count));
    });
}

pub(super) fn show_data_take(ui: &mut egui::Ui, state: &mut ViewState) {
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

pub(super) fn show_data(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("data", |ui| {
        let (bytes_len, bytes_preview) = state
            .schema
            .data
            .bytes
            .read(|data| (data.len(), preview_slice(data)));
        ui.label("Data<u8>: root.data.bytes");
        ui.label(format!("len = {bytes_len}, preview = {bytes_preview}"));

        ui.separator();

        let (samples_len, samples_preview) = state
            .schema
            .data
            .samples
            .read(|data| (data.len(), preview_f32_slice(data)));
        ui.label("Data<f32>: root.data.samples");
        ui.label(format!("len = {samples_len}, preview = {samples_preview}"));

        ui.separator();

        // The header and the buffer are read together, so they always match.
        let (summary, measurement_preview) =
            state
                .schema
                .data
                .measurement
                .read_with_header(|data, header| {
                    let summary = format!(
                        "{}: {} samples at {} Hz in {}",
                        header.label,
                        data.len(),
                        header.sample_rate,
                        header.unit
                    );
                    (summary, preview_f32_slice(data))
                });
        ui.label("Data<f32, Measurement>: root.data.measurement");
        ui.label(summary);
        ui.label(format!("preview = {measurement_preview}"));

        ui.separator();

        let (buffer_len, buffer_preview) = state
            .schema
            .data
            .nested
            .buffer
            .read(|data| (data.len(), preview_slice(data)));
        ui.label("Nested Data<u16>: root.data.nested.buffer");
        ui.label(format!("len = {buffer_len}, preview = {buffer_preview}"));
    });
}

pub(super) fn show_multi_data(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("multi data", |ui| {
        let mut bytes_items = state.schema.multi_data.bytes.read_all(|data| {
            data.iter()
                .map(|(index, values)| (*index, values.len(), preview_slice(values)))
                .collect::<Vec<_>>()
        });
        bytes_items.sort_by_key(|(index, _, _)| *index);
        ui.label("DataMulti<u8>: root.multi_data.bytes");
        show_multi_data_preview(ui, &bytes_items);

        ui.separator();

        let mut samples_items = state.schema.multi_data.samples.read_all(|data| {
            data.iter()
                .map(|(index, values)| (*index, values.len(), preview_f32_slice(values)))
                .collect::<Vec<_>>()
        });
        samples_items.sort_by_key(|(index, _, _)| *index);
        ui.label("DataMulti<f32>: root.multi_data.samples");
        show_multi_data_preview(ui, &samples_items);

        ui.separator();

        let mut buffer_items = state.schema.multi_data.nested.buffer.read_all(|data| {
            data.iter()
                .map(|(index, values)| (*index, values.len(), preview_slice(values)))
                .collect::<Vec<_>>()
        });
        buffer_items.sort_by_key(|(index, _, _)| *index);
        ui.label("Nested DataMulti<u16>: root.multi_data.nested.buffer");
        show_multi_data_preview(ui, &buffer_items);
    });
}

pub(super) fn show_multi_data_take(ui: &mut egui::Ui, state: &mut ViewState) {
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
