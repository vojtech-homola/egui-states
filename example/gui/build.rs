use egui_states::build_scripts::generate_python;

#[path = "src/states.rs"]
#[allow(dead_code)]
mod states;

use states::States;

fn main() {
    println!("cargo:rerun-if-changed=../gui/src/states.rs");

    generate_python::<States>("../python/states_server.py").unwrap();
}
