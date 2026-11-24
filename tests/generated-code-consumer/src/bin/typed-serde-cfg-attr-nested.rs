// A `serde` attribute behind two conditions, after another attribute.
#[egui_states::typed]
enum Invalid {
    #[cfg_attr(all(), allow(dead_code), cfg_attr(all(), serde(skip)))]
    Hidden,
    Value(u8),
}
fn main() {}
