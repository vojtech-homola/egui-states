fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../build-artifacts/egui_states_test_bindings");
    egui_states::build_scripts::generate_python::<egui_states_test_schema::State>(root).unwrap();
}
