use egui_states::build_scripts::generate_rust;
use gui_core::State;

fn main() {
    println!("cargo:rerun-if-changed=../gui-core/src/");

    generate_rust::<State>("./src/states_server.rs").unwrap();
}
