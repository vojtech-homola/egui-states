use super::ViewState;

pub(super) fn show_value_vec(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("vec state", |ui| {
        ui.label("VecState<i32>: root.value_vec.items");
        ui.label(
            "Collection is display-only here. Buttons emit signals and the server mutates the data.",
        );

        state.schema.value_vec.items.read(|list| {
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
                state.schema.value_vec.actions.append_item.emit(());
            }
            if ui.button("remove last").clicked() {
                state.schema.value_vec.actions.remove_last.emit(());
            }
            if ui.button("reset demo").clicked() {
                state.schema.value_vec.actions.reset_demo.emit(());
            }
        });
    });
}

pub(super) fn show_value_map(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("map state", |ui| {
        ui.label("MapState<u16, u32>: root.value_map.items");
        ui.label(
            "Collection is display-only here. Buttons emit signals and the server mutates the data.",
        );

        let mut items: Vec<_> = state.schema.value_map.items.get().into_iter().collect();
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
                state.schema.value_map.actions.insert_next.emit(());
            }
            if ui.button("remove lowest").clicked() {
                state.schema.value_map.actions.remove_lowest.emit(());
            }
            if ui.button("reset demo").clicked() {
                state.schema.value_map.actions.reset_demo.emit(());
            }
        });
    });
}
