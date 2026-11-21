// `Home {}` sends the same as `Home`; only the unit form is accepted.
#[egui_states::typed]
enum Invalid {
    Home {},
    Go(u8),
}
fn main() {}
