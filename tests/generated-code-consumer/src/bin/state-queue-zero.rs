#[derive(egui_states::State)]
struct Invalid {
    #[state(queue = 0)]
    events: egui_states::ValueTake<u8>,
}
fn main() {}
