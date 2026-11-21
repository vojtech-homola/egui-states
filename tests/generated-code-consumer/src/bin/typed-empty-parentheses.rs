// `Reset()` sends the same as `Reset`; only the unit form is accepted.
#[egui_states::typed]
enum Invalid {
    Reset(),
    Go(u8),
}
fn main() {}
