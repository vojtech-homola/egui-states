// Describing a type that contains itself would never end.
#[egui_states::typed]
enum Invalid {
    Leaf,
    Sequence(Vec<Invalid>),
}
fn main() {}
