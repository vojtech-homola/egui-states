#[derive(egui_states::State)]
struct Invalid {
    #[state(queue = 4)]
    count: egui_states::Value<u8>,
}
fn main() {}
