#[derive(egui_states::State)]
struct Invalid {
    #[state(queue = 10u8)]
    events: egui_states::ValueTake<u8>,
}
fn main() {}
