use egui_states::build_scripts::{generate_python_wrapper, generate_pytypes, generate_rust_server};
use gui::States;

fn main() {
    println!("cargo:rerun-if-changed=../gui/src/states.rs");
    println!("cargo:rerun-if-changed=../gui/src/enums.rs");
    println!("cargo:rerun-if-changed=../gui/src/custom.rs");

    generate_rust_server::<States>("./src/states.rs").unwrap();

    generate_python_wrapper::<States>("states_server/states.py", None::<(String, String)>).unwrap();

    generate_pytypes::<States>("states_server/core.pyi").unwrap();
}
