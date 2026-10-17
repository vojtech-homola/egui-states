#[egui_states::typed]
struct Inner {
    value: u8,
}
#[egui_states::typed]
struct Invalid {
    #[serde(flatten)]
    inner: Inner,
}
fn main() {}
