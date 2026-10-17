// A skipped field is still described by `Typed`, so peers disagree on the wire.
#[egui_states::typed]
struct Invalid {
    value: u8,
    #[serde(skip)]
    hidden: u8,
}
fn main() {}
