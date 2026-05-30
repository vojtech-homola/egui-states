use egui_states::{MapState, Signal, VecState};

use super::State;

#[derive(egui_states::State)]
pub(super) struct ValueVecActionStates {
    pub append_item: Signal<()>,
    pub remove_last: Signal<()>,
    pub reset_demo: Signal<()>,
}

#[derive(egui_states::State)]
pub(super) struct ValueVecStates {
    pub items: VecState<i32>,
    pub actions: ValueVecActionStates,
}

#[derive(egui_states::State)]
pub(super) struct ValueMapActionStates {
    pub insert_next: Signal<()>,
    pub remove_lowest: Signal<()>,
    pub reset_demo: Signal<()>,
}

#[derive(egui_states::State)]
pub(super) struct ValueMapStates {
    pub items: MapState<u16, u32>,
    pub actions: ValueMapActionStates,
}

pub(super) fn show_value_vec(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("value vec", |ui| {
        ui.label("ValueVec<i32>: root.value_vec.items");
        ui.label(
            "Collection is display-only here. Buttons emit signals and Python mutates the data.",
        );

        state.value_vec.items.read(|list| {
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
                state.value_vec.actions.append_item.set(());
            }
            if ui.button("remove last").clicked() {
                state.value_vec.actions.remove_last.set(());
            }
            if ui.button("reset demo").clicked() {
                state.value_vec.actions.reset_demo.set(());
            }
        });
    });
}

pub(super) fn show_value_map(ui: &mut egui::Ui, state: &mut State) {
    ui.collapsing("value map", |ui| {
        ui.label("ValueMap<u16, u32>: root.value_map.items");
        ui.label(
            "Collection is display-only here. Buttons emit signals and Python mutates the data.",
        );

        let mut items: Vec<_> = state.value_map.items.get().into_iter().collect();
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
                state.value_map.actions.insert_next.set(());
            }
            if ui.button("remove lowest").clicked() {
                state.value_map.actions.remove_lowest.set(());
            }
            if ui.button("reset demo").clicked() {
                state.value_map.actions.reset_demo.set(());
            }
        });
    });
}
