use egui_states::build_scripts::generate_python;

use gui_core::State;

fn main() {
    println!("cargo:rerun-if-changed=../gui-core/src/");

    generate_python::<State>("../python/states_server.py").unwrap();
}
