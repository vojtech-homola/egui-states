// Describing a type that contains itself would never end.
#[egui_states::typed]
struct Invalid {
    children: Vec<Invalid>,
}
fn main() {}
