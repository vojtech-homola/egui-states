use egui_states::build_script::python;

#[path = "src/states.rs"]
mod states;

use states::States;

fn main() {
    println!("cargo:rerun-if-changed=../gui/src/states.rs");

    python::generate::<States>("../states-server/states.py").unwrap();
}
