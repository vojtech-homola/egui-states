#[derive(egui_states::State)]
struct Invalid {
    #[state(init = 1)]
    #[state(init = 2)]
    count: egui_states::Value<i32>,
}
fn main() {}
