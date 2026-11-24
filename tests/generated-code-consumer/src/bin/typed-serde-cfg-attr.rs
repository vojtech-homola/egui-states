// The attribute would change the encoding whenever the condition holds.
#[egui_states::typed]
struct Invalid {
    value: u8,
    #[cfg_attr(all(), serde(skip))]
    hidden: u8,
}
fn main() {}
