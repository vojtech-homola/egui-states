// The field is described in every build, also in one that does not have it.
#[egui_states::typed]
struct Invalid {
    value: u8,
    #[cfg(any())]
    extra: u8,
}
fn main() {}
