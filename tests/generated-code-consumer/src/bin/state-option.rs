use egui_states::Image;

#[derive(egui_states::State)]
struct Invalid {
    #[state(fast)]
    image: Image,
}
fn main() {}
