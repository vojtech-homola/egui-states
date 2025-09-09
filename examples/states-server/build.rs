use egui_states_pyserver::build::{self, parse_states_for_client, parse_states_for_server};

fn main() {
    println!("cargo:rerun-if-changed=../gui/src/states.rs");
    println!("cargo:rerun-if-changed=../gui/src/enums.rs");
    println!("cargo:rerun-if-changed=../gui/src/custom.rs");

    parse_states_for_server(
        "../gui/src/states.rs",
        "src/states.rs",
        "State",
        &None,
        &None,
        Vec::new(),
    )
    .unwrap();

    parse_states_for_client(
        "../gui/src/states.rs",
        "states_server/states.py",
        "States",
        None,
        None,
    )
    .unwrap();

    build::write_annotation("states_server/core.pyi".to_string(), None, None);
}
