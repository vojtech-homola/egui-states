//! A test protocol independent of the examples, with observable callback results.
use egui_states::{
    DataMultiTake, DataTake, MapState, Queue, Signal, Static, Value, ValueTake, VecState,
};

#[derive(egui_states::State)]
pub struct IntegrationState {
    pub value: Value<i32>,
    pub wire: Value<crate::sections::WirePacket, Queue>,
    pub wire_echo: Static<crate::sections::WirePacket>,
    pub callback_value: Static<i32>,
    pub phase: Static<u32>,
    pub command: Signal<u32, Queue>,
    pub items: VecState<i32>,
    pub map: MapState<u16, u32>,
    pub take: ValueTake<String>,
    pub empty: ValueTake<()>,
    pub data: DataTake<u8>,
    pub multi: DataMultiTake<u16>,
    pub cached: DataTake<u8>,
    pub cached_multi: DataMultiTake<u16>,
}
