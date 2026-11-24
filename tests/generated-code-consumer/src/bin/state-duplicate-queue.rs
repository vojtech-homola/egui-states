#[derive(egui_states::State)]
struct Invalid {
    #[state(queue, queue = 4)]
    events: egui_states::ValueTake<u8>,
}
fn main() {}
