use egui_states::{MapState, Signal, VecState};

#[derive(egui_states::State)]
pub struct ValueVecActionStates {
    pub append_item: Signal<()>,
    pub remove_last: Signal<()>,
    pub reset_demo: Signal<()>,
}

#[derive(egui_states::State)]
pub struct ValueVecStates {
    pub items: VecState<i32>,
    pub actions: ValueVecActionStates,
}

#[derive(egui_states::State)]
pub struct ValueMapActionStates {
    pub insert_next: Signal<()>,
    pub remove_lowest: Signal<()>,
    pub reset_demo: Signal<()>,
}

#[derive(egui_states::State)]
pub struct ValueMapStates {
    pub items: MapState<u16, u32>,
    pub actions: ValueMapActionStates,
}
