// The variant is described in every build, also in one that does not have it.
#[egui_states::typed]
enum Invalid {
    Always,
    #[cfg(all())]
    Sometimes(u8),
}
fn main() {}
