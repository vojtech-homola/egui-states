// A skipped variant field is still described by `Typed`.
#[egui_states::typed]
enum Invalid {
    Go {
        value: u8,
        #[serde(skip)]
        hidden: u8,
    },
}
fn main() {}
