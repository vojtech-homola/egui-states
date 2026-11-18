#[derive(egui_states::State)]
struct Invalid {
    #[state(init = ())]
    reset: egui_states::Signal<()>,
}
fn main() {}
