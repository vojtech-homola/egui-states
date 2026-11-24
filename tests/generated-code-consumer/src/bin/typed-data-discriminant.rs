// The wire carries a variant's position, never its discriminant.
#[egui_states::typed]
#[repr(u8)]
enum Invalid {
    Stop = 1,
    Go(u8) = 2,
}
fn main() {}
