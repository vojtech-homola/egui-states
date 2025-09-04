use egui_states_pyserver::build::{self, parse_states_for_client, parse_states_for_server};

fn main() {
    println!("cargo:rerun-if-changed=../images_gui/src/states.rs");
    // println!("cargo:rerun-if-changed=../gui/src/enums.rs");
    // println!("cargo:rerun-if-changed=../gui/src/custom.rs");

    // let enums = Some(build::read_enums("../gui/src/enums.rs"));
    // let custom = Some(build::read_structs("../gui/src/custom.rs"));
    // let replace = vec!["enums".to_string(), "custom".to_string()];
    parse_states_for_server(
        "../images_gui/src/states.rs",
        "src/states.rs",
        "State",
        &None,
        &None,
        Vec::new(),
    )
    .unwrap();

    parse_states_for_client(
        "../images_gui/src/states.rs",
        "images/states.py",
        "States",
        None,
        None,
    )
    .unwrap();

    build::write_annotation("images/core.pyi".to_string(), None, None);
}