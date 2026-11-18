use egui_states::Value;

#[derive(egui_states::State)]
struct Invalid {
    #[state(low_priority)]
    value: Value<u32>,
}
fn main() {}
