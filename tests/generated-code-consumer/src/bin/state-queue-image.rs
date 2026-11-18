use egui_states::Image;

#[derive(egui_states::State)]
struct Invalid {
    #[state(queue)]
    image: Image,
}
fn main() {}
